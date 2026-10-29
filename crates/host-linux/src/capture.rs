//! PipeWire oqimidan kadr olish va H.264 ga kodlash (GStreamer).

use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;
use protocol::video_reason;
use std::{
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use tokio::sync::mpsc;

/// Kodlagichlar ustuvorlik tartibida: apparat tezlatish birinchi.
const ENCODERS: [&str; 4] = ["vah264enc", "nvh264enc", "x264enc", "openh264enc"];

pub struct Frame {
    pub keyframe: bool,
    pub pts_us: u64,
    pub data: Vec<u8>,
}

pub struct Capture {
    pipeline: gst::Pipeline,
    pub frames: mpsc::Receiver<Frame>,
    pub errors: mpsc::UnboundedReceiver<String>,
    pub encoder: &'static str,
    stop: Arc<AtomicBool>,
    _fd: OwnedFd,
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

pub struct CaptureError {
    pub reason: u8,
    pub error: anyhow::Error,
}

fn fail(reason: u8, error: anyhow::Error) -> CaptureError {
    CaptureError { reason, error }
}

fn encoder_desc(factory: &str, kbps: u32, gop: u32) -> String {
    match factory {
        "vah264enc" => format!("vah264enc bitrate={kbps} key-int-max={gop}"),
        "nvh264enc" => format!("nvh264enc bitrate={kbps} gop-size={gop}"),
        "x264enc" => format!(
            "x264enc tune=zerolatency speed-preset=ultrafast bitrate={kbps} key-int-max={gop} byte-stream=true"
        ),
        _ => format!("openh264enc bitrate={} gop-size={gop}", kbps * 1000),
    }
}

pub fn start(fd: OwnedFd, node_id: u32, fps: u32, kbps: u32, max_height: u32) -> Result<Capture, CaptureError> {
    gst::init()
        .context("GStreamer ishga tushmadi")
        .map_err(|e| fail(video_reason::NO_CAPTURE, e))?;

    for need in ["pipewiresrc", "videoconvert", "videoscale", "videorate"] {
        if gst::ElementFactory::find(need).is_none() {
            return Err(fail(
                video_reason::NO_CAPTURE,
                anyhow!("`{need}` GStreamer elementi topilmadi (sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-base)"),
            ));
        }
    }
    let forced = std::env::var("RD_ENCODER").ok();
    let encoder = ENCODERS
        .iter()
        .copied()
        .find(|e| forced.as_deref().map_or(true, |f| f == *e) && gst::ElementFactory::find(e).is_some())
        .ok_or_else(|| {
            fail(
                video_reason::NO_ENCODER,
                anyhow!("H.264 kodlagich topilmadi (sudo apt install gstreamer1.0-plugins-ugly yoki gstreamer1.0-plugins-bad)"),
            )
        })?;

    // Kalit kadr har 1 soniyada: Viewer ulanganda tez boshlanishi uchun.
    let gop = fps.max(1);
    let height_caps = if max_height > 0 {
        format!(",height={max_height}")
    } else {
        String::new()
    };
    let desc = format!(
        "pipewiresrc fd={} path={} do-timestamp=true ! videoconvert ! videoscale ! videorate ! \
         video/x-raw,framerate={}/1{} ! {} ! video/x-h264,stream-format=byte-stream,alignment=au ! \
         appsink name=sink sync=false max-buffers=4 drop=true",
        fd.as_raw_fd(),
        node_id,
        fps,
        height_caps,
        encoder_desc(encoder, kbps, gop)
    );

    let pipeline = gst::parse::launch(&desc)
        .context("pipeline yaratilmadi")
        .map_err(|e| fail(video_reason::NO_CAPTURE, e))?
        .downcast::<gst::Pipeline>()
        .map_err(|_| fail(video_reason::NO_CAPTURE, anyhow!("pipeline turi noto'g'ri")))?;
    let sink = pipeline
        .by_name("sink")
        .ok_or_else(|| fail(video_reason::NO_CAPTURE, anyhow!("appsink topilmadi")))?
        .downcast::<gst_app::AppSink>()
        .map_err(|_| fail(video_reason::NO_CAPTURE, anyhow!("appsink turi noto'g'ri")))?;

    let (tx, rx) = mpsc::channel::<Frame>(8);
    // Kanal to'lsa (Viewer sekin), keyingi kalit kadrgacha kadrlarni tashlaymiz,
    // shunda dekoder buzilgan oqim olmaydi.
    let dropping = Arc::new(AtomicBool::new(false));
    sink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_sample(move |sink| {
                let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                let keyframe = !buffer.flags().contains(gst::BufferFlags::DELTA_UNIT);
                if dropping.load(Ordering::Relaxed) && !keyframe {
                    return Ok(gst::FlowSuccess::Ok);
                }
                let frame = Frame {
                    keyframe,
                    pts_us: buffer.pts().map(|t| t.useconds()).unwrap_or(0),
                    data: map.as_slice().to_vec(),
                };
                match tx.try_send(frame) {
                    Ok(()) => dropping.store(false, Ordering::Relaxed),
                    Err(mpsc::error::TrySendError::Full(_)) => dropping.store(true, Ordering::Relaxed),
                    Err(mpsc::error::TrySendError::Closed(_)) => return Err(gst::FlowError::Flushing),
                }
                Ok(gst::FlowSuccess::Ok)
            })
            .build(),
    );

    let bus = pipeline
        .bus()
        .ok_or_else(|| fail(video_reason::NO_CAPTURE, anyhow!("bus yo'q")))?;
    let stop = Arc::new(AtomicBool::new(false));
    let (err_tx, err_rx) = mpsc::unbounded_channel::<String>();
    {
        let stop = stop.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::SeqCst) {
                let msg = bus.timed_pop_filtered(
                    gst::ClockTime::from_mseconds(200),
                    &[gst::MessageType::Error, gst::MessageType::Eos],
                );
                if let Some(msg) = msg {
                    match msg.view() {
                        gst::MessageView::Error(e) => {
                            let _ = err_tx.send(format!("{} ({:?})", e.error(), e.debug()));
                            break;
                        }
                        gst::MessageView::Eos(_) => {
                            let _ = err_tx.send("oqim tugadi (EOS)".to_string());
                            break;
                        }
                        _ => {}
                    }
                }
            }
        });
    }

    pipeline
        .set_state(gst::State::Playing)
        .context("pipeline ishga tushmadi")
        .map_err(|e| fail(video_reason::NO_CAPTURE, e))?;

    Ok(Capture {
        pipeline,
        frames: rx,
        errors: err_rx,
        encoder,
        stop,
        _fd: fd,
    })
}
