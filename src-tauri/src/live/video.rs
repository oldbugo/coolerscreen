//! Animated background: a clip decoded by the bundled ffmpeg as raw RGBA frames at the target size, looping forever.
//! The reader thread keeps only the newest decoded frame; the renderer copies it each tick.
use super::{LH, LW};
use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

const FRAME_BYTES: usize = LW * LH * 4;

pub struct VideoBg {
    pub latest: Arc<Mutex<Option<Arc<Vec<u8>>>>>,
    stop: Arc<AtomicBool>,
    /// Raw process handle of ffmpeg (for CPU/RAM measurement).
    pub child_handle: Arc<Mutex<Option<usize>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl VideoBg {
    /// `fps`: frame rate ffmpeg delivers (paced in real time with -re).
    pub fn start(clip: &Path, fps: u32) -> Result<VideoBg, String> {
        let ffmpeg = crate::media::ffmpeg::locate();
        if !ffmpeg.exists() {
            return Err(format!("ffmpeg not found at {}", ffmpeg.display()));
        }
        if !clip.exists() {
            return Err(format!("background clip not found: {}", clip.display()));
        }
        let latest = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let handle = Arc::new(Mutex::new(None));
        let (l2, s2, h2, clip) = (latest.clone(), stop.clone(), handle.clone(), clip.to_path_buf());
        let thread = std::thread::spawn(move || {
            while !s2.load(Ordering::Relaxed) {
                let Some(mut child) = spawn(&ffmpeg, &clip, fps) else {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    continue;
                };
                crate::media::sys::adopt_child(&child);
                #[cfg(windows)]
                {
                    use std::os::windows::io::AsRawHandle;
                    *h2.lock().unwrap() = Some(child.as_raw_handle() as usize);
                }
                let mut out = child.stdout.take().unwrap();
                loop {
                    let mut buf = vec![0u8; FRAME_BYTES];
                    if s2.load(Ordering::Relaxed) || out.read_exact(&mut buf).is_err() {
                        break;
                    }
                    *l2.lock().unwrap() = Some(Arc::new(buf));
                }
                *h2.lock().unwrap() = None;
                let _ = child.kill();
                let _ = child.wait();
                if !s2.load(Ordering::Relaxed) {
                    std::thread::sleep(std::time::Duration::from_secs(1));
                }
            }
        });
        Ok(VideoBg { latest, stop, child_handle: handle, thread: Some(thread) })
    }

    pub fn frame(&self) -> Option<Arc<Vec<u8>>> {
        self.latest.lock().unwrap().clone()
    }
}

fn spawn(ffmpeg: &Path, clip: &Path, fps: u32) -> Option<Child> {
    let mut c = Command::new(ffmpeg);
    c.args(["-hide_banner", "-loglevel", "error", "-re", "-stream_loop", "-1", "-i"])
        .arg(clip)
        .args(["-an", "-vf"])
        .arg(format!("fps={fps},scale={LW}:{LH}:force_original_aspect_ratio=increase,crop={LW}:{LH},format=rgba"))
        .args(["-f", "rawvideo", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    c.spawn().ok()
}

impl Drop for VideoBg {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Killing ffmpeg unblocks the reader's read_exact.
        crate::media::sys::kill_adopted_children();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
