// Prevent console window in addition to Slint window in Windows release builds when, e.g., starting the app via file manager. Ignored on other platforms.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use slint::{Color, SharedString, ModelRc, VecModel};
use std::rc::Rc;

use std::error::Error;

slint::include_modules!();

use anyhow::{bail, Result};

use gst::prelude::*;

fn try_gstreamer_video_frame_to_pixel_buffer(
    frame: &gst_video::VideoFrame<gst_video::video_frame::Readable>,
) -> Result<slint::SharedPixelBuffer<slint::Rgb8Pixel>> {
    match frame.format() {
        gst_video::VideoFormat::Rgb => {
            let mut slint_pixel_buffer =
                slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(frame.width(), frame.height());
            frame
                .buffer()
                .copy_to_slice(0, slint_pixel_buffer.make_mut_bytes())
                .expect("Unable to copy to slice!"); // Copies!
            Ok(slint_pixel_buffer)
        }
        _ => {
            bail!(
                "Cannot convert frame to a slint RGB frame because it is format {}",
                frame.format().to_str()
            )
        }
    }
}

fn setup_gauge(style: &mut CircularGaugeStyle, warning_min: Option<i32>, labels: Option<Vec<&str>>) {
    let mut tick_marks = Vec::new();
    for i in (style.min_value..=style.max_value).step_by(style.tick_interval as usize) {
        let angle = (i - style.min_value) as f32 / (style.max_value as f32 - style.min_value as f32) * (style.max_angle - style.min_angle) + style.min_angle;
        let label = match labels {
            Some(ref l) => if i < l.len() as i32 { SharedString::from(l[i as usize]) } else { SharedString::new() },
            None => if style.label_interval == 0 {
                 SharedString::new()
                 } else {
                     match i % style.label_interval {
                        0 => SharedString::from(format!("{}", i)),
                        _ => SharedString::new(),
                    }
                }
        };
        let color = match warning_min {
            Some(warning_min) => if i >= warning_min { Color::from_argb_f32(1.0, 1.0, 0.0, 0.0) } else { Color::from_argb_f32(1.0, 1.0, 1.0, 1.0) },
            None => Color::from_argb_f32(1.0, 1.0, 1.0, 1.0),
        };
        tick_marks.push(TickMark {
            angle,
            label,
            color,
        });
    }
    style.tick_marks = ModelRc::from(Rc::new(VecModel::from(tick_marks)).clone());

    let mut sub_tick_marks = Vec::new();
    if style.sub_tick_interval > 0 {
        for i in (style.min_value..=style.max_value).step_by(style.sub_tick_interval as usize) {
            let angle = (i - style.min_value) as f32 / (style.max_value as f32 - style.min_value as f32) * (style.max_angle - style.min_angle) + style.min_angle;
            sub_tick_marks.push(SubTickMark { angle: angle });
        }
    }
    style.sub_tick_marks = ModelRc::from(Rc::new(VecModel::from(sub_tick_marks)).clone());
}

fn main() -> Result<(), Box<dyn Error>> {
    let ui = AppWindow::new()?;
    let app_weak = ui.as_weak();

    let mut accelerometer_style = ui.get_accelerometer_style();
    setup_gauge(&mut accelerometer_style, None, None);
    ui.set_accelerometer_style(accelerometer_style);

    let mut tachometer_style = ui.get_tachometer_style();
    setup_gauge(&mut tachometer_style, Some(7), None);
    ui.set_tachometer_style(tachometer_style);

    let mut fuel_gauge_style = ui.get_fuel_gauge_style();
    setup_gauge(&mut fuel_gauge_style, None, Some(vec!["E", "", "", "", "F"]));
    ui.set_fuel_gauge_style(fuel_gauge_style);

    let mut temp_gauge_style = ui.get_temp_gauge_style();
    setup_gauge(&mut temp_gauge_style, None, Some(vec!["C", "", "", "", "H"]));
    ui.set_temp_gauge_style(temp_gauge_style);

    gst::init().unwrap();
    let source = gst::ElementFactory::make("v4l2src")
        .name("source")
        .build()
        .expect("Could not create source element.");
    source.set_property("device", &"/dev/video0");

    let videoconvert = gst::ElementFactory::make("videoconvert")
        .name("convert")
        .build()
        .expect("Failed to create videoconvert element");

    let caps = gst::Caps::builder("video/x-raw")
        .field("format", &"RGB")
        .field("width", 1280)
        .field("height", 720)
        .build();

    let capsfilter = gst::ElementFactory::make("capsfilter")
        .name("filter")
        .build()
        .expect("Failed to create capsfilter element");
    capsfilter.set_property("caps", &caps);

    let width: u32 = 1280;
    let height: u32 = 720;

    let appsink = gst_app::AppSink::builder()
        .caps(
            &gst_video::VideoCapsBuilder::new()
                .format(gst_video::VideoFormat::Rgb)
                .width(width as i32)
                .height(height as i32)
                .build(),
        )
        .build();

    let pipeline = gst::Pipeline::with_name("test-pipeline");

    pipeline.add_many([&source, &videoconvert, &capsfilter, &appsink.upcast_ref()]).unwrap();
    gst::Element::link_many([&source, &videoconvert, &capsfilter, &appsink.upcast_ref()]).unwrap();

    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |appsink| {
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer_owned().unwrap(); // Probably copies!
                let video_info =
                    gst_video::VideoInfo::builder(gst_video::VideoFormat::Rgb, width, height)
                        .build()
                        .expect("couldn't build video info!");
                let video_frame =
                    gst_video::VideoFrame::from_buffer_readable(buffer, &video_info).unwrap();
                let slint_frame = try_gstreamer_video_frame_to_pixel_buffer(&video_frame)
                    .expect("Unable to convert the video frame to a slint video frame!");

                app_weak
                    .upgrade_in_event_loop(|app| {
                        app.set_video_frame(slint::Image::from_rgb8(slint_frame))
                    })
                    .unwrap();

                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    pipeline
        .set_state(gst::State::Playing)
        .expect("Unable to set the pipeline to the `Playing` state");

    ui.run()?;

    Ok(())
}
