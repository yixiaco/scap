use std::error::Error;
use std::sync::mpsc;
use std::{cmp};
use crate::{
    capturer::{Options, CGSize, CGPoint, CGRect, Resolution},
    frame::{BGRAFrame, Frame},
    device::display::{self},
};
use windows::{
    Wdk::System::SystemServices::OkControl, 
    Win32::Graphics::Gdi::{
        GetMonitorInfoW, 
        HMONITOR, 
        MONITORINFOEXW
    }
};
use std::time::{SystemTime, UNIX_EPOCH};
use windows_capture::{
    capture::{CaptureControl, WindowsCaptureHandler},
    frame::Frame as Wframe,
    graphics_capture_api::{GraphicsCaptureApi, InternalCaptureControl},
    monitor::Monitor,
    settings::{ColorFormat, Settings},
    window::Window,
};

#[derive(Debug)]
struct Capturer {
    pub tx: mpsc::Sender<Frame>,
    pub crop: Option<CGRect>,
}

impl Capturer {
    pub fn new(tx: mpsc::Sender<Frame>) -> Self {
        println!("I am here inside impl_capturer_new");
        Capturer { tx, crop: None }
    }

    pub fn with_crop(mut self, crop: Option<CGRect>) -> Self {
        self.crop = crop;
        self
    }
}

pub struct WinStream {
    settings: Settings<FlagStruct>,
    capture_control: Option<CaptureControl<Capturer, Box<dyn std::error::Error + Send + Sync>>>,
}

impl WindowsCaptureHandler for Capturer {
    type Flags = FlagStruct;
    type Error = Box<dyn std::error::Error + Send + Sync>;

    fn new(flagValues: Self::Flags) -> Result<Self, Self::Error> {
        println!("I am here inside WindowsCaptureHandler new");
        Ok(Self { tx:flagValues.tx, crop:flagValues.crop })
    }

    fn on_frame_arrived(
        &mut self,
        mut frame: &mut Wframe,
        _: InternalCaptureControl,
    ) -> Result<(), Self::Error> {

        match &self.crop {
            Some(cropped_area) => {

                // get the cropped area
                let start_x = cropped_area.origin.x as u32;
                let start_y = cropped_area.origin.y as u32;
                let end_x = (cropped_area.origin.x + cropped_area.size.width) as u32;
                let end_y = (cropped_area.origin.y + cropped_area.size.height) as u32;

                // crop the frame
                let mut cropped_buffer = frame.buffer_crop(start_x, start_y, end_x, end_y)
                    .expect("Failed to crop buffer");

                println!("Frame Arrived: {}x{} and padding = {}",
                    cropped_buffer.width(),
                    cropped_buffer.height(),
                    cropped_buffer.has_padding(),
                );

                // get raw frame buffer
                let raw_frame_buffer = match cropped_buffer.as_raw_nopadding_buffer() {
                    Ok(buffer) => buffer,
                    Err(_) => return Err(("Failed to get raw buffer").into()),
    
                };

                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Failed to get current time")
                    .as_nanos() as u64;

                let bgr_frame = BGRAFrame {
                    display_time: current_time,
                    width: cropped_area.size.width as i32,
                    height: cropped_area.size.height as i32,
                    data: raw_frame_buffer.to_vec(),
                };

                self.tx.send(Frame::BGRA(bgr_frame))
                    .expect("Failed to send data");
            }
            None => {
                println!("Frame Arrived: {}x{}",
                    frame.width(),
                    frame.height(),
                );

                // get raw frame buffer
                let mut frame_buffer = frame.buffer().unwrap();
                let raw_frame_buffer = frame_buffer.as_raw_buffer();
                let frame_data = raw_frame_buffer.to_vec();
                let current_time = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .expect("Failed to get current time")
                    .as_nanos() as u64;
                let bgr_frame = BGRAFrame {
                    display_time: current_time,
                    width: frame.width() as i32,
                    height: frame.height() as i32,
                    data: frame_data,
                };

                self.tx.send(Frame::BGRA(bgr_frame))
                    .expect("Failed to send data");
            }
        }
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        println!("Closed");
        Ok(())
    }
}

impl WinStream {
    pub fn start_capture(&mut self) {

        let capture_control = Capturer::start_free_threaded(self.settings.clone()).unwrap();
        self.capture_control = Some(capture_control);
    }

    pub fn stop_capture(&mut self) {
        let capture_control = self.capture_control.take().unwrap();
        let _ = capture_control.stop();
    }
}

#[derive(Clone, Debug)]
struct FlagStruct {
    pub tx: mpsc::Sender<Frame>,
    pub crop: Option<CGRect>,
}

pub fn create_capturer(
    options: &Options,
    tx: mpsc::Sender<Frame>,
) -> WinStream {
    let settings = Settings::new(
        Monitor::primary().unwrap(),
        Some(true),
        None,
        ColorFormat::Bgra8,
        FlagStruct { tx, crop: Some(get_source_rect(options))},
    
    ).unwrap();

    return WinStream {
        settings,
        capture_control: None,
    };
}

pub fn get_output_frame_size(options: &Options) -> [u32; 2] {
    let source_rect = get_source_rect(options);

    let mut output_width = source_rect.size.width as u32;
    let mut output_height = source_rect.size.height as u32;

    match options.output_resolution {
        Resolution::Captured => {}
        _ => {
            let [resolved_width, resolved_height] = options
                .output_resolution
                .value((source_rect.size.width as f32) / (source_rect.size.height as f32));
            output_width = cmp::min(output_width, resolved_width);
            output_height = cmp::min(output_height, resolved_height);
        }
    }

    if output_width % 2 == 1 {
        output_width -= 1;
    }

    if output_height % 2 == 1 {
        output_height -= 1;
    }
    println!("Output frame size: [{}, {}]", output_width, output_height);
    [output_width, output_height]
}

pub fn get_source_rect(options: &Options) -> CGRect {
    let display = display::get_main_display();
    let width_result = display.width();
    let height_result = display.height();

    let width: u32 = match width_result {
        Ok(val) => val,
        Err(_) => 0,
    };
    let height = match height_result {
        Ok(val) => val,
        Err(_) => 0,
    };

    let source_rect = match &options.source_rect {
        Some(val) => {
            let input_width = if (val.size.width as i64) % 2 == 0 {
                val.size.width as i64
            } else {
                (val.size.width as i64) + 1
            };
            let input_height = if (val.size.height as i64) % 2 == 0 {
                val.size.height as i64
            } else {
                (val.size.height as i64) + 1
            };
            CGRect {
                origin: CGPoint {
                    x: val.origin.x,
                    y: val.origin.y,
                },
                size: CGSize {
                    width: input_width as f64,
                    height: input_height as f64,
                },
            }
        }
        None => CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: width as f64,
                height: height as f64,
            },
        },
    };

    source_rect
}
