//! Optional per-iteration tracing for the EST beam-search algorithm.
//!
//! A scheduler run can be instrumented with an [`EstTraceSink`]. The default
//! [`NoopTraceSink`] is a single virtual call per event, so leaving the trace
//! enabled at the type level is cheap when no sink is attached.
//!
//! [`JsonlTraceSink`] writes one JSON object per line to a file alongside the
//! schedule output, suitable for offline analysis or backend ingestion.

use serde::Serialize;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Discrete events emitted during an EST run.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EstTraceEvent {
    /// Emitted once at the start of the run.
    Started {
        /// Identifier of the scheduling algorithm; always `"est"`. Lets
        /// downstream tooling tag traces without inspecting the shape.
        algorithm: &'static str,
        k_beams: usize,
        branching_factor: usize,
        endangered_threshold: u32,
        fom: String,
        task_count: usize,
        block_count: usize,
        horizon_start_mjd: f64,
        horizon_end_mjd: f64,
    },

    /// Emitted after every beam-search round, post-pruning.
    IterationCompleted {
        round: u32,
        live_beams_in: usize,
        children_generated: usize,
        terminal_added: usize,
        kept: usize,
        /// Surviving beam scores in this round, sorted descending.
        beam_scores: Vec<f64>,
        best_score: Option<f64>,
        median_score: Option<f64>,
        worst_score: Option<f64>,
        scheduled_in_best: Option<usize>,
        wall_ms: u128,
    },

    /// Emitted once at the end of the run.
    Summary {
        total_rounds: u32,
        terminal_count: usize,
        best_score: f64,
        best_scheduled_count: usize,
        wall_ms_total: u128,
    },
}

/// A destination for [`EstTraceEvent`] records.
pub trait EstTraceSink: std::fmt::Debug + Send + Sync {
    /// Record a single event. Implementations should be cheap and never panic.
    fn record(&self, event: &EstTraceEvent);
}

/// A sink that discards every event. Used as the implicit default.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTraceSink;

impl EstTraceSink for NoopTraceSink {
    fn record(&self, _event: &EstTraceEvent) {}
}

/// A sink that serializes each event as one JSON line to a file.
#[derive(Debug)]
pub struct JsonlTraceSink {
    path: PathBuf,
    writer: Mutex<BufWriter<File>>,
}

impl JsonlTraceSink {
    /// Open `path` for write, truncating any existing file.
    pub fn create(path: impl Into<PathBuf>) -> std::io::Result<Self> {
        let path = path.into();
        let file = File::create(&path)?;
        Ok(Self {
            path,
            writer: Mutex::new(BufWriter::new(file)),
        })
    }

    /// Path the sink writes to.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl EstTraceSink for JsonlTraceSink {
    fn record(&self, event: &EstTraceEvent) {
        let Ok(mut w) = self.writer.lock() else {
            return;
        };
        match serde_json::to_string(event) {
            Ok(line) => {
                let _ = writeln!(w, "{line}");
                let _ = w.flush();
            }
            Err(err) => {
                log::warn!("est trace: failed to serialize event: {err}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[derive(Debug, Default)]
    struct VecSink {
        events: Mutex<Vec<EstTraceEvent>>,
    }

    impl EstTraceSink for VecSink {
        fn record(&self, event: &EstTraceEvent) {
            self.events.lock().unwrap().push(event.clone());
        }
    }

    #[test]
    fn noop_sink_does_nothing() {
        let sink = NoopTraceSink;
        sink.record(&EstTraceEvent::Summary {
            total_rounds: 0,
            terminal_count: 0,
            best_score: 0.0,
            best_scheduled_count: 0,
            wall_ms_total: 0,
        });
    }

    #[test]
    fn jsonl_sink_writes_lines() {
        let dir = std::env::temp_dir();
        let path = dir.join("est_trace_test.jsonl");
        let sink: Arc<dyn EstTraceSink> = Arc::new(JsonlTraceSink::create(&path).unwrap());
        sink.record(&EstTraceEvent::Started {
            algorithm: "est",
            k_beams: 1,
            branching_factor: 1,
            endangered_threshold: 0,
            fom: "soft_constraint".into(),
            task_count: 0,
            block_count: 0,
            horizon_start_mjd: 0.0,
            horizon_end_mjd: 1.0,
        });
        sink.record(&EstTraceEvent::Summary {
            total_rounds: 0,
            terminal_count: 0,
            best_score: 0.0,
            best_scheduled_count: 0,
            wall_ms_total: 0,
        });
        drop(sink);
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.lines().count(), 2);
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn vec_sink_collects_events() {
        let sink = Arc::new(VecSink::default());
        let dyn_sink: Arc<dyn EstTraceSink> = sink.clone();
        dyn_sink.record(&EstTraceEvent::Summary {
            total_rounds: 1,
            terminal_count: 1,
            best_score: 0.5,
            best_scheduled_count: 2,
            wall_ms_total: 10,
        });
        let events = sink.events.lock().unwrap();
        assert_eq!(events.len(), 1);
    }
}
