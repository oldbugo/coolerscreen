//! Bounded, payload-free diagnostics for the Live USB transition sequence.
//! Records stay in memory until the sequence finishes, preserving USB spacing.
use crate::device::DevErr;
use serde::Serialize;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

type Sink = Arc<dyn Fn(Value) + Send + Sync>;
static SINK: OnceLock<Sink> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
const MAX_STEPS: usize = 8;
const MAX_MS: u128 = 120_000;

/// Install one process-lifetime sink. Standalone sessions have no sink or new log output.
/// The embedding host owns the diagnostics opt-out and private event delivery.
pub fn install_sink(sink: Sink) {
    let _ = SINK.set(sink);
}

#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Flow {
    Connect,
    Stop,
    StopFast,
    CancelConnect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Step {
    Open,
    EndStream,
    Info,
    PreserveEnding,
    EnterLive,
    DisplayOn,
    LoopClip,
    ScreenOff,
    Cancelled,
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Ok,
    Cancelled,
    NotFound,
    DescriptorFailure,
    Busy,
    NoReply,
    WriteTimeout,
    ShortWrite,
    TransportError,
}

fn classify(error: &DevErr) -> Outcome {
    match error.code.as_deref() {
        Some("notFound") => Outcome::NotFound,
        Some("failedUsb") => Outcome::DescriptorFailure,
        Some("busy") => Outcome::Busy,
        Some("noReply") => Outcome::NoReply,
        _ => {
            let message = error.message.to_ascii_lowercase();
            if message.contains("paused") {
                Outcome::Cancelled
            } else if message.contains("timed out") || message.contains("timeout") {
                Outcome::WriteTimeout
            } else if message.contains("short write") {
                Outcome::ShortWrite
            } else {
                Outcome::TransportError
            }
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Entry {
    step: Step,
    result: Outcome,
    elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Record<'a> {
    schema: u8,
    sequence: u64,
    operation: Flow,
    total_ms: u64,
    steps: &'a [Entry],
}

pub(super) struct Trace {
    sequence: u64,
    flow: Flow,
    started: Instant,
    entries: Vec<Entry>,
}

fn millis(duration: Duration) -> u64 {
    duration.as_millis().min(MAX_MS) as u64
}

impl Trace {
    pub fn new(flow: Flow) -> Self {
        Self {
            sequence: SEQUENCE.fetch_add(1, Ordering::Relaxed),
            flow,
            started: Instant::now(),
            entries: Vec::with_capacity(MAX_STEPS),
        }
    }

    pub fn run<T>(
        &mut self,
        step: Step,
        operation: impl FnOnce() -> Result<T, DevErr>,
    ) -> Result<T, DevErr> {
        let started = Instant::now();
        let result = operation();
        if self.entries.len() < MAX_STEPS {
            self.entries.push(Entry {
                step,
                result: result
                    .as_ref()
                    .map_or_else(|e| classify(e), |_| Outcome::Ok),
                elapsed_ms: millis(started.elapsed()),
            });
        }
        result
    }

    pub(super) fn value(&self) -> Value {
        serde_json::to_value(Record {
            schema: 1,
            sequence: self.sequence,
            operation: self.flow,
            total_ms: millis(self.started.elapsed()),
            steps: &self.entries,
        })
        .expect("finite Live diagnostic record")
    }

    pub fn flush(self) {
        if let Some(sink) = SINK.get() {
            sink(self.value());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trace_is_bounded_and_never_serializes_error_text_or_ids() {
        let mut trace = Trace::new(Flow::Connect);
        for _ in 0..20 {
            let _ = trace.run(Step::Info, || -> Result<(), DevErr> {
                Err(DevErr::new(
                    "secret-device-id private-file token USB write timed out",
                ))
            });
        }
        assert_eq!(trace.entries.len(), MAX_STEPS);
        let line = trace.value().to_string();
        assert!(line.len() < 1600);
        assert!(!line.contains("secret-device-id"));
        assert!(!line.contains("private-file"));
        assert!(!line.contains("token"));
        assert!(line.contains("write_timeout"));
        assert_eq!(millis(Duration::from_secs(u64::MAX)), MAX_MS as u64);
    }

    #[test]
    fn every_error_is_classified_to_a_finite_result() {
        for (code, message, expected) in [
            (Some("notFound"), "anything", Outcome::NotFound),
            (Some("failedUsb"), "anything", Outcome::DescriptorFailure),
            (Some("busy"), "anything", Outcome::Busy),
            (Some("noReply"), "anything", Outcome::NoReply),
            (None, "Live connection paused", Outcome::Cancelled),
            (None, "USB write timed out", Outcome::WriteTimeout),
            (None, "USB short write", Outcome::ShortWrite),
            (
                Some("unexpected-user-id"),
                "arbitrary-private-message",
                Outcome::TransportError,
            ),
        ] {
            assert_eq!(
                classify(&DevErr {
                    code: code.map(str::to_owned),
                    message: message.into(),
                    failed_usb: false,
                }),
                expected
            );
        }
    }

    #[test]
    fn elapsed_time_measures_the_operation_and_the_record_has_only_finite_fields() {
        let mut trace = Trace::new(Flow::Stop);
        trace
            .run(Step::EndStream, || {
                std::thread::sleep(Duration::from_millis(3));
                Ok(())
            })
            .unwrap();
        let value = trace.value();
        assert_eq!(value.as_object().unwrap().len(), 5);
        assert_eq!(value["schema"], 1);
        assert_eq!(value["operation"], "stop");
        let elapsed = value["steps"][0]["elapsedMs"].as_u64().unwrap();
        assert!(elapsed >= 3 && elapsed <= MAX_MS as u64);
        assert!(value["totalMs"].as_u64().unwrap() >= elapsed);
        assert_eq!(value["steps"][0].as_object().unwrap().len(), 3);
    }
}
