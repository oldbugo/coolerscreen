//! The live link to the screen over WinUSB (via the device layer). Sequence, as the vendor app does it:
//! connect: FF D9 FF D9 (end any old stream), 0x15 [00 0A] (stored: do not switch the display off after live), 0x11 (enter live).
//! Frames: u32LE length | JPEG | u16LE sum. Keepalive 0x11 every 1.5 s. stop: FF D9 FF D9, display on (0x0E [1]),
//! loop clip (0x14 [1,0]), so the 2.0 behaviour (the loop clip keeps playing) holds when the companion stops.
use crate::device::{DevErr, Screen};
use std::time::{Duration, Instant};

#[path = "session_trace.rs"]
pub mod trace;
use trace::{Flow, Step, Trace};

pub const KEEPALIVE: Duration = Duration::from_millis(1500);
const END_OF_STREAM: [u8; 4] = [0xff, 0xd9, 0xff, 0xd9];

/// How a live stream ends.
/// LoopClip (default, the 2.0 behaviour): display on and the stored loop clip plays.
/// ScreenOff (opt-in): the vendor app's shutdown sequence, the screen turns off (0x14 [0,0]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Ending {
    #[default]
    LoopClip,
    ScreenOff,
}

pub struct LiveSession {
    scr: Option<Screen>,
    last_keep: Instant,
    wire: Vec<u8>,
}

impl LiveSession {
    pub fn connect() -> Result<LiveSession, String> {
        Self::connect_checked(&|| false).map_err(|e| e.to_string())
    }

    pub fn connect_checked(cancelled: &dyn Fn() -> bool) -> Result<LiveSession, DevErr> {
        let mut trace = Trace::new(Flow::Connect);
        let result = connect_sequence(
            || Screen::open_usb().map_err(crate::device::ops::live_open_error),
            cancelled,
            &mut trace,
        );
        trace.flush();
        let scr = result?;
        Ok(LiveSession {
            scr: Some(scr),
            last_keep: Instant::now(),
            wire: Vec::new(),
        })
    }

    /// 0x11 when 1.5 s have passed since the last one (call between frames, never mid-frame).
    pub fn keepalive_if_due(&mut self) -> Result<(), String> {
        if self.last_keep.elapsed() < KEEPALIVE {
            return Ok(());
        }
        let scr = self.scr.as_mut().ok_or("session closed")?;
        scr.live_key(0x11, &[]).map_err(|e| e.to_string())?;
        scr.live_drain();
        self.last_keep = Instant::now();
        Ok(())
    }

    pub fn send_frame(&mut self, jpeg: &[u8]) -> Result<(), String> {
        self.keepalive_if_due()?;
        wrap_frame_into(jpeg, &mut self.wire);
        let scr = self.scr.as_mut().ok_or("session closed")?;
        scr.live_write(&self.wire).map_err(|e| e.to_string())
    }

    /// Orderly end with the default ending: the loop clip plays, display on.
    pub fn stop(self) {
        self.stop_with(Ending::LoopClip);
    }

    /// Orderly end. FF D9 FF D9 is always its own transfer; the screen-off command follows as a separate transfer a moment
    /// later (the vendor notes the firmware rejects the two when they are coalesced).
    pub fn stop_with(mut self, ending: Ending) {
        if let Some(scr) = self.scr.take() {
            let mut trace = Trace::new(Flow::Stop);
            stop_sequence(scr, ending, StopMode::Normal, &mut trace);
            trace.flush();
        }
    }

    /// Shutdown variant: the same writes with only a 2 ms gap, no settling waits. Completes in well under a second.
    pub fn stop_fast(mut self, ending: Ending) {
        if let Some(scr) = self.scr.take() {
            let mut trace = Trace::new(Flow::StopFast);
            stop_sequence(scr, ending, StopMode::Fast, &mut trace);
            trace.flush();
        }
    }

    /// Drop the link without the end sequence (the device is gone or erroring).
    pub fn abandon(mut self) {
        if let Some(scr) = self.scr.take() {
            scr.close();
        }
    }

    /// A scheduler hold arrived at the end of the bounded handshake. Release promptly; best-effort
    /// hand-back uses short transfers so the handshake plus cancellation stays within the hold wait.
    pub fn cancel_connect(mut self) {
        if let Some(scr) = self.scr.take() {
            let mut trace = Trace::new(Flow::CancelConnect);
            stop_sequence(scr, Ending::LoopClip, StopMode::Cancel, &mut trace);
            trace.flush();
        }
    }
}

/// A private sequence adapter: production delegates to the same Screen methods;
/// tests record calls and replace waits without opening a device.
trait SessionOps: Sized {
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> Result<(), DevErr>;
    fn info(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), DevErr>;
    fn key(&mut self, key: u8, payload: &[u8]) -> Result<(), DevErr>;
    fn screen_off(&mut self) -> Result<(), DevErr>;
    fn wait(&mut self, duration: Duration) {
        std::thread::sleep(duration);
    }
    fn close(self);
}

impl SessionOps for Screen {
    fn write(&mut self, bytes: &[u8], timeout: Duration) -> Result<(), DevErr> {
        self.live_write_timeout(bytes, timeout)
    }
    fn info(&mut self, cancelled: &dyn Fn() -> bool) -> Result<(), DevErr> {
        self.live_info(cancelled)
    }
    fn key(&mut self, key: u8, payload: &[u8]) -> Result<(), DevErr> {
        self.live_key(key, payload)
    }
    fn screen_off(&mut self) -> Result<(), DevErr> {
        self.live_screen_off()
    }
    fn close(self) {
        Screen::close(self);
    }
}

fn check_cancelled(cancelled: &dyn Fn() -> bool, trace: &mut Trace) -> Result<(), DevErr> {
    if cancelled() {
        trace.run(Step::Cancelled, || {
            Err(DevErr::new("Live connection paused"))
        })
    } else {
        Ok(())
    }
}

fn connect_sequence<O: SessionOps>(
    open: impl FnOnce() -> Result<O, DevErr>,
    cancelled: &dyn Fn() -> bool,
    trace: &mut Trace,
) -> Result<O, DevErr> {
    let mut scr = trace.run(Step::Open, open)?;
    let result = (|| -> Result<(), DevErr> {
        check_cancelled(cancelled, trace)?;
        trace.run(Step::EndStream, || {
            scr.write(&END_OF_STREAM, Duration::from_millis(500))
        })?;
        scr.wait(Duration::from_millis(100));
        trace.run(Step::Info, || scr.info(cancelled))?;
        check_cancelled(cancelled, trace)?;
        trace.run(Step::PreserveEnding, || {
            scr.write(
                &crate::device::frame::frame(0x15, &[0x00, 0x0a]),
                Duration::from_millis(500),
            )
        })?;
        scr.wait(Duration::from_millis(100));
        check_cancelled(cancelled, trace)?;
        trace.run(Step::EnterLive, || {
            scr.write(
                &crate::device::frame::frame(0x11, &[]),
                Duration::from_millis(500),
            )
        })
    })();
    if let Err(error) = result {
        let _ = trace.run(Step::Close, || {
            scr.close();
            Ok(())
        });
        return Err(crate::device::friendly(error));
    }
    Ok(scr)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StopMode {
    Normal,
    Fast,
    Cancel,
}

fn stop_sequence<O: SessionOps>(mut scr: O, ending: Ending, mode: StopMode, trace: &mut Trace) {
    let timeout = if mode == StopMode::Cancel {
        Duration::from_millis(100)
    } else {
        Duration::from_secs(1)
    };
    let _ = trace.run(Step::EndStream, || scr.write(&END_OF_STREAM, timeout));
    if mode == StopMode::Fast {
        scr.wait(Duration::from_millis(2));
    }
    match ending {
        Ending::LoopClip => {
            if mode == StopMode::Normal {
                scr.wait(Duration::from_millis(200));
            }
            let _ = trace.run(Step::DisplayOn, || {
                if mode == StopMode::Cancel {
                    scr.write(&crate::device::frame::frame(0x0e, &[1]), timeout)
                } else {
                    scr.key(0x0e, &[1])
                }
            });
            if mode == StopMode::Normal {
                scr.wait(Duration::from_millis(100));
            }
            let _ = trace.run(Step::LoopClip, || {
                if mode == StopMode::Cancel {
                    scr.write(&crate::device::frame::frame(0x14, &[1, 0]), timeout)
                } else {
                    scr.key(0x14, &[1, 0])
                }
            });
        }
        Ending::ScreenOff => {
            if mode == StopMode::Normal {
                scr.wait(Duration::from_millis(2));
            }
            let _ = trace.run(Step::ScreenOff, || scr.screen_off());
        }
    }
    if mode == StopMode::Normal {
        scr.wait(Duration::from_millis(300));
    }
    let _ = trace.run(Step::Close, || {
        scr.close();
        Ok(())
    });
}

/// `u32LE len | JPEG | u16LE sum`, sum = (4 length bytes + JPEG bytes) & 0xFFFF.
pub fn wrap_frame_into(jpeg: &[u8], out: &mut Vec<u8>) {
    out.clear();
    out.reserve(jpeg.len() + 6);
    out.extend_from_slice(&(jpeg.len() as u32).to_le_bytes());
    out.extend_from_slice(jpeg);
    let sum = out.iter().fold(0u32, |a, &b| a.wrapping_add(b as u32)) & 0xffff;
    out.extend_from_slice(&(sum as u16).to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Call {
        Open,
        Write(Vec<u8>, Duration),
        Info,
        Wait(Duration),
        Close,
    }

    struct FakeOps {
        calls: Arc<Mutex<Vec<Call>>>,
        failure: Option<Step>,
    }

    fn failure() -> DevErr {
        DevErr {
            code: Some("noReply".into()),
            message: "original private failure".into(),
            failed_usb: false,
        }
    }

    impl SessionOps for FakeOps {
        fn write(&mut self, bytes: &[u8], timeout: Duration) -> Result<(), DevErr> {
            self.calls
                .lock()
                .unwrap()
                .push(Call::Write(bytes.to_vec(), timeout));
            let step = if bytes == END_OF_STREAM {
                Step::EndStream
            } else {
                match bytes[4] {
                    0x15 => Step::PreserveEnding,
                    0x11 => Step::EnterLive,
                    0x0e => Step::DisplayOn,
                    0x14 if bytes[5] == 1 => Step::LoopClip,
                    0x14 => Step::ScreenOff,
                    _ => panic!("unexpected fake write"),
                }
            };
            if self.failure == Some(step) {
                Err(failure())
            } else {
                Ok(())
            }
        }
        fn info(&mut self, _: &dyn Fn() -> bool) -> Result<(), DevErr> {
            self.calls.lock().unwrap().push(Call::Info);
            if self.failure == Some(Step::Info) {
                Err(failure())
            } else {
                Ok(())
            }
        }
        fn key(&mut self, key: u8, payload: &[u8]) -> Result<(), DevErr> {
            self.write(
                &crate::device::frame::frame(key, payload),
                Duration::from_secs(1),
            )
        }
        fn screen_off(&mut self) -> Result<(), DevErr> {
            self.key(0x14, &[0, 0])
        }
        fn wait(&mut self, duration: Duration) {
            self.calls.lock().unwrap().push(Call::Wait(duration));
        }
        fn close(self) {
            self.calls.lock().unwrap().push(Call::Close);
        }
    }

    fn written(key: u8, payload: &[u8], millis: u64) -> Call {
        Call::Write(
            crate::device::frame::frame(key, payload),
            Duration::from_millis(millis),
        )
    }

    fn stop_calls(ending: Ending, mode: StopMode) -> Vec<Call> {
        let millis = if mode == StopMode::Cancel { 100 } else { 1000 };
        let mut calls = vec![Call::Write(
            END_OF_STREAM.to_vec(),
            Duration::from_millis(millis),
        )];
        if mode == StopMode::Fast {
            calls.push(Call::Wait(Duration::from_millis(2)));
        }
        match ending {
            Ending::LoopClip => {
                if mode == StopMode::Normal {
                    calls.push(Call::Wait(Duration::from_millis(200)));
                }
                calls.push(written(0x0e, &[1], millis));
                if mode == StopMode::Normal {
                    calls.push(Call::Wait(Duration::from_millis(100)));
                }
                calls.push(written(0x14, &[1, 0], millis));
            }
            Ending::ScreenOff => {
                if mode == StopMode::Normal {
                    calls.push(Call::Wait(Duration::from_millis(2)));
                }
                calls.push(written(0x14, &[0, 0], millis));
            }
        }
        if mode == StopMode::Normal {
            calls.push(Call::Wait(Duration::from_millis(300)));
        }
        calls.push(Call::Close);
        calls
    }

    #[test]
    fn connect_success_preserves_commands_timeouts_waits_and_records_each_step() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut trace = Trace::new(Flow::Connect);
        let scr = connect_sequence(
            || {
                calls.lock().unwrap().push(Call::Open);
                Ok(FakeOps {
                    calls: calls.clone(),
                    failure: None,
                })
            },
            &|| false,
            &mut trace,
        )
        .unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                Call::Open,
                Call::Write(END_OF_STREAM.to_vec(), Duration::from_millis(500)),
                Call::Wait(Duration::from_millis(100)),
                Call::Info,
                written(0x15, &[0, 10], 500),
                Call::Wait(Duration::from_millis(100)),
                written(0x11, &[], 500),
            ]
        );
        let record = trace.value();
        assert_eq!(record["operation"], "connect");
        assert_eq!(
            record["steps"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| s["step"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![
                "open",
                "end_stream",
                "info",
                "preserve_ending",
                "enter_live"
            ]
        );
        assert!(record["steps"]
            .as_array()
            .unwrap()
            .iter()
            .all(|s| s["result"] == "ok"));
        // A successful handshake retains the open handle until the caller ends it.
        scr.close();
    }

    #[test]
    fn connect_failure_at_each_step_stops_forward_progress_closes_and_preserves_error() {
        let steps = [
            Step::Open,
            Step::EndStream,
            Step::Info,
            Step::PreserveEnding,
            Step::EnterLive,
        ];
        for (index, step) in steps.into_iter().enumerate() {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mut trace = Trace::new(Flow::Connect);
            let result = connect_sequence(
                || {
                    calls.lock().unwrap().push(Call::Open);
                    if step == Step::Open {
                        Err(failure())
                    } else {
                        Ok(FakeOps {
                            calls: calls.clone(),
                            failure: Some(step),
                        })
                    }
                },
                &|| false,
                &mut trace,
            );
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("failure was lost"),
            };
            assert_eq!(error.code, failure().code);
            assert_eq!(error.message, failure().message);
            let record = trace.value();
            let entries = record["steps"].as_array().unwrap();
            assert_eq!(entries[index]["result"], "no_reply");
            assert_eq!(entries.len(), index + 1 + usize::from(index > 0));
            if index > 0 {
                assert_eq!(entries.last().unwrap()["step"], "close");
                assert_eq!(calls.lock().unwrap().last(), Some(&Call::Close));
            } else {
                assert_eq!(*calls.lock().unwrap(), vec![Call::Open]);
            }
            assert!(!record.to_string().contains("original private failure"));
        }
    }

    #[test]
    fn cancellation_at_each_existing_checkpoint_closes_without_next_write() {
        for checkpoint in 0..3 {
            let checks = std::cell::Cell::new(0);
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mut trace = Trace::new(Flow::Connect);
            let result = connect_sequence(
                || {
                    Ok(FakeOps {
                        calls: calls.clone(),
                        failure: None,
                    })
                },
                &|| {
                    let check = checks.get();
                    checks.set(check + 1);
                    check == checkpoint
                },
                &mut trace,
            );
            let error = match result {
                Err(error) => error,
                Ok(_) => panic!("cancellation lost"),
            };
            assert_eq!(error.message, "Live connection paused");
            let record = trace.value();
            let entries = record["steps"].as_array().unwrap();
            assert_eq!(entries[entries.len() - 2]["step"], "cancelled");
            assert_eq!(entries[entries.len() - 2]["result"], "cancelled");
            assert_eq!(entries.last().unwrap()["step"], "close");
            assert_eq!(calls.lock().unwrap().last(), Some(&Call::Close));
            assert!(!calls.lock().unwrap().contains(&written(0x11, &[], 500)));
        }
    }

    #[test]
    fn all_stop_paths_preserve_order_waits_timeouts_and_continue_after_every_write_failure() {
        for (ending, mode, flow) in [
            (Ending::LoopClip, StopMode::Normal, Flow::Stop),
            (Ending::ScreenOff, StopMode::Normal, Flow::Stop),
            (Ending::LoopClip, StopMode::Fast, Flow::StopFast),
            (Ending::ScreenOff, StopMode::Fast, Flow::StopFast),
            (Ending::LoopClip, StopMode::Cancel, Flow::CancelConnect),
        ] {
            let failures = if ending == Ending::LoopClip {
                vec![
                    None,
                    Some(Step::EndStream),
                    Some(Step::DisplayOn),
                    Some(Step::LoopClip),
                ]
            } else {
                vec![None, Some(Step::EndStream), Some(Step::ScreenOff)]
            };
            for failure_step in failures {
                let calls = Arc::new(Mutex::new(Vec::new()));
                let mut trace = Trace::new(flow);
                stop_sequence(
                    FakeOps {
                        calls: calls.clone(),
                        failure: failure_step,
                    },
                    ending,
                    mode,
                    &mut trace,
                );
                assert_eq!(*calls.lock().unwrap(), stop_calls(ending, mode));
                let record = trace.value();
                let entries = record["steps"].as_array().unwrap();
                assert_eq!(entries.last().unwrap()["step"], "close");
                assert_eq!(
                    entries.iter().filter(|s| s["result"] != "ok").count(),
                    usize::from(failure_step.is_some())
                );
            }
        }
    }

    #[test]
    fn frame_layout() {
        let mut v = Vec::new();
        wrap_frame_into(&[0xff, 0xd8, 0xff, 0xd9], &mut v);
        assert_eq!(v, vec![0x04, 0, 0, 0, 0xff, 0xd8, 0xff, 0xd9, 0xb3, 0x03]);
    }
}
