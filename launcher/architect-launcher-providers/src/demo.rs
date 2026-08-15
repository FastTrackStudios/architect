//! Demo provider with sample data for testing the launcher UI.
//!
//! Provides fake entries across multiple tag categories so you can see
//! the full UI (sidebar, tags, filter chips, etc.) without needing
//! real system data.

use architect_launcher_core::{ActivationResult, Item, Provider, ProviderConfig};

pub struct DemoProvider {
    config: ProviderConfig,
}

impl DemoProvider {
    pub fn new() -> Self {
        Self {
            config: ProviderConfig {
                name: "demo".into(),
                icon: "D".into(),
                prefix: Some('d'),
                ..Default::default()
            },
        }
    }
}

impl Default for DemoProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl Provider for DemoProvider {
    fn name(&self) -> &str {
        "demo"
    }

    fn config(&self) -> &ProviderConfig {
        &self.config
    }

    fn config_mut(&mut self) -> &mut ProviderConfig {
        &mut self.config
    }

    fn query(
        &self,
        _query: &str,
        _exact: bool,
    ) -> Result<Vec<Item>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(demo_items())
    }

    fn activate(
        &self,
        item: &Item,
        _action: &str,
    ) -> Result<ActivationResult, Box<dyn std::error::Error + Send + Sync>> {
        tracing::info!(item = %item.label, "Demo item activated");
        Ok(ActivationResult::Close)
    }
}

fn demo_items() -> Vec<Item> {
    vec![
        // Audio effects
        Item::new("demo-reverb", "ReaVerbate", "demo")
            .with_sub("Cockos reverb plugin")
            .with_icon("R")
            .with_tags(&["audio/effects/reverb", "audio/effects/plugin/vst"])
            .with_search_fields(vec!["ReaVerbate".into(), "reverb".into(), "cockos".into()]),
        Item::new("demo-eq", "ReaEQ", "demo")
            .with_sub("Cockos parametric equalizer")
            .with_icon("E")
            .with_tags(&["audio/effects/eq", "audio/effects/plugin/vst"])
            .with_search_fields(vec!["ReaEQ".into(), "equalizer".into(), "cockos".into()]),
        Item::new("demo-comp", "ReaComp", "demo")
            .with_sub("Cockos compressor")
            .with_icon("C")
            .with_tags(&["audio/effects/dynamics", "audio/effects/plugin/vst"])
            .with_search_fields(vec![
                "ReaComp".into(),
                "compressor".into(),
                "dynamics".into(),
            ]),
        Item::new("demo-delay", "ReaDelay", "demo")
            .with_sub("Cockos multi-tap delay")
            .with_icon("D")
            .with_tags(&["audio/effects/delay", "audio/effects/plugin/vst"])
            .with_search_fields(vec!["ReaDelay".into(), "delay".into(), "echo".into()]),
        Item::new("demo-gate", "ReaGate", "demo")
            .with_sub("Cockos noise gate")
            .with_icon("G")
            .with_tags(&["audio/effects/dynamics", "audio/effects/plugin/vst"])
            .with_search_fields(vec!["ReaGate".into(), "gate".into(), "noise".into()]),
        // Instruments
        Item::new("demo-synth", "Vital", "demo")
            .with_sub("Spectral warping wavetable synth")
            .with_icon("V")
            .with_tags(&["audio/instruments/synth", "audio/effects/plugin/vst3"])
            .with_search_fields(vec![
                "Vital".into(),
                "synthesizer".into(),
                "wavetable".into(),
            ]),
        Item::new("demo-sampler", "ReaSamplomatic5000", "demo")
            .with_sub("Cockos sampler")
            .with_icon("S")
            .with_tags(&["audio/instruments/sampler", "audio/effects/plugin/vst"])
            .with_search_fields(vec![
                "ReaSamplomatic5000".into(),
                "sampler".into(),
                "cockos".into(),
            ]),
        Item::new("demo-piano", "Piano One", "demo")
            .with_sub("Free acoustic piano")
            .with_icon("P")
            .with_tags(&["audio/instruments/piano", "audio/effects/plugin/vst"])
            .with_search_fields(vec!["Piano One".into(), "piano".into(), "acoustic".into()]),
        // Reaper actions
        Item::new("demo-action-save", "Save Project", "demo")
            .with_sub("Action: File > Save project")
            .with_icon("A")
            .with_tags(&["reaper/actions/file"])
            .with_search_fields(vec!["Save Project".into(), "save".into(), "file".into()]),
        Item::new("demo-action-render", "Render Project", "demo")
            .with_sub("Action: File > Render project to disk")
            .with_icon("A")
            .with_tags(&["reaper/actions/file"])
            .with_search_fields(vec![
                "Render Project".into(),
                "render".into(),
                "bounce".into(),
                "export".into(),
            ]),
        Item::new("demo-action-play", "Play/Stop", "demo")
            .with_sub("Action: Transport > Play/Stop")
            .with_icon("A")
            .with_tags(&["reaper/actions/transport"])
            .with_search_fields(vec![
                "Play Stop".into(),
                "transport".into(),
                "playback".into(),
            ]),
        Item::new("demo-action-record", "Record", "demo")
            .with_sub("Action: Transport > Record")
            .with_icon("A")
            .with_tags(&["reaper/actions/transport"])
            .with_search_fields(vec!["Record".into(), "transport".into(), "arm".into()]),
        // Tracks
        Item::new("demo-track-master", "Master Track", "demo")
            .with_sub("Track: Master bus")
            .with_icon("T")
            .with_tags(&["reaper/tracks"])
            .with_search_fields(vec!["Master Track".into(), "master".into(), "bus".into()]),
        Item::new("demo-track-drums", "Drums", "demo")
            .with_sub("Track: Drum bus (folder)")
            .with_icon("T")
            .with_tags(&["reaper/tracks"])
            .with_search_fields(vec!["Drums".into(), "drum".into(), "percussion".into()]),
        Item::new("demo-track-vocals", "Lead Vocals", "demo")
            .with_sub("Track: Lead vocal recording")
            .with_icon("T")
            .with_tags(&["reaper/tracks"])
            .with_search_fields(vec!["Lead Vocals".into(), "vocal".into(), "voice".into()]),
    ]
}
