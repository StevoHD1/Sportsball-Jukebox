//! Listeners that turn MIDI and OSC input into [`ControlMessage`]s.
//!
//! Each listener runs on its own thread (CoreMIDI's, or a UDP thread for
//! OSC) and hands every message straight to a [`Sink`], which the app
//! points at `Session::handle_control`. Dropping a listener stops it.

pub mod midi;
pub mod osc;
pub mod remote;

use std::sync::Arc;

pub use sbj_session::ControlMessage;

/// Where listeners deliver messages. Called from listener threads.
pub type Sink = Arc<dyn Fn(ControlMessage) + Send + Sync>;
