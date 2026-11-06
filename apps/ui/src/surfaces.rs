//! The lab's ports and control-surface targets, for extensions (#311;
//! `architect_extensions::ports`).
//!
//! - **Ports**: OSC over UDP, and a virtual MIDI port, `Lab Surface`,
//!   whose device end is the lab's own ([`lab_ports`]); a system MIDI
//!   backend is an app's to add (`midir` behind `architect_input::PortBackend`).
//! - **Targets** ([`lab_targets`]): the metronome's tempo
//!   (`lab.metronome.tempo`, 40–240 BPM, kept by the lab: the metronome
//!   has no tempo of its own yet) and the theme's accent hue (`lab.theme.accent_hue`, 0–360°,
//!   the `brand` token on the `Pick` layer, its saturation and lightness
//!   kept).
//!
//! `lab.surface` (`apps/sandbox/extensions/surface`, `ARCHITECT_LAB_SURFACE=1`)
//! maps an OSC surface onto both.

use std::sync::{Arc, Mutex, PoisonError};

use architect_input::{
    PortKind, Ports, Target, TargetKind, TargetSink, Targets, UdpOsc, VirtualDevice, VirtualPorts,
};
use architect_theme::{LayerKind, Rgba, Subscription, ThemeStore};

/// The lab's virtual MIDI port.
pub const LAB_SURFACE: &str = "Lab Surface";

/// The lab's ports: OSC over UDP and its virtual MIDI port (with its
/// device end, which a demo or a test drives).
#[must_use]
pub fn lab_ports() -> (Ports, VirtualDevice) {
    let midi = VirtualPorts::new(PortKind::Midi);
    let device = midi.add(LAB_SURFACE);
    (Ports::new().with_midi(midi).with_osc(UdpOsc), device)
}

/// The lab's targets: the metronome's tempo and, given the theme, its
/// accent's hue.
#[must_use]
pub fn lab_targets(themes: Option<ThemeStore>) -> Targets {
    let targets = Targets::new();
    targets.register(Tempo {
        bpm: Arc::new(Mutex::new(120.0)),
    });
    if let Some(store) = themes {
        targets.register(AccentHue {
            store,
            watching: Arc::new(Mutex::new(None)),
        });
    }
    targets
}

/// The metronome's tempo.
struct Tempo {
    bpm: Arc<Mutex<f64>>,
}

impl Target for Tempo {
    fn kind(&self) -> TargetKind {
        TargetKind {
            id: "lab.metronome.tempo".to_owned(),
            title: "Metronome tempo".to_owned(),
            min: 40.0,
            max: 240.0,
            step: Some(1.0),
            count: 1,
        }
    }

    fn read(&self, index: u32) -> Option<f64> {
        (index == 0).then(|| *self.bpm.lock().unwrap_or_else(PoisonError::into_inner))
    }

    fn write(&self, index: u32, value: f64) -> Result<(), String> {
        if index != 0 {
            return Err("one tempo".to_owned());
        }
        *self.bpm.lock().unwrap_or_else(PoisonError::into_inner) = value.round();
        Ok(())
    }

    fn label(&self, _index: u32) -> Option<String> {
        self.read(0).map(|bpm| format!("{bpm:.0} BPM"))
    }
}

/// The token that is the theme's accent as a person sees it (the palette's
/// accent; `accent` itself is architect-ui's hover wash).
const ACCENT: &str = "brand";

/// The theme's accent, by its hue.
struct AccentHue {
    store: ThemeStore,
    /// The store's listener while a surface observes it.
    watching: Arc<Mutex<Option<Subscription>>>,
}

fn accent(store: &ThemeStore) -> Option<Rgba> {
    store.current().get(ACCENT).and_then(Rgba::parse)
}

/// `(hue°, saturation, lightness)` of `colour`.
fn hsl(colour: Rgba) -> (f64, f64, f64) {
    let (red, green, blue) = (
        f64::from(colour.r),
        f64::from(colour.g),
        f64::from(colour.b),
    );
    let (max, min) = (red.max(green).max(blue), red.min(green).min(blue));
    let lightness = f64::midpoint(max, min);
    let chroma = max - min;
    if chroma < 1e-9 {
        return (0.0, 0.0, lightness);
    }
    let saturation = chroma / (1.0 - 2.0f64.mul_add(lightness, -1.0).abs());
    let sector = if (max - red).abs() < 1e-9 {
        ((green - blue) / chroma).rem_euclid(6.0)
    } else if (max - green).abs() < 1e-9 {
        (blue - red) / chroma + 2.0
    } else {
        (red - green) / chroma + 4.0
    };
    (sector * 60.0, saturation, lightness)
}

impl Target for AccentHue {
    fn kind(&self) -> TargetKind {
        TargetKind {
            id: "lab.theme.accent_hue".to_owned(),
            title: "Accent hue".to_owned(),
            min: 0.0,
            max: 360.0,
            step: Some(2.0),
            count: 1,
        }
    }

    fn read(&self, index: u32) -> Option<f64> {
        (index == 0).then(|| accent(&self.store).map_or(0.0, |c| hsl(c).0))
    }

    fn write(&self, index: u32, value: f64) -> Result<(), String> {
        if index != 0 {
            return Err("one accent".to_owned());
        }
        let (_, s, l) = accent(&self.store).map_or((0.0, 0.7, 0.5), hsl);
        // A grey accent has no hue to turn: give it some colour.
        let s = if s < 0.05 { 0.7 } else { s };
        let css = format!(
            "hsl({:.0}, {:.0}%, {:.0}%)",
            value.rem_euclid(360.0),
            s * 100.0,
            l * 100.0
        );
        self.store.set_token(LayerKind::Pick, ACCENT, &css);
        Ok(())
    }

    fn observe(&self, changed: TargetSink) {
        let sub = self.store.subscribe(move |theme| {
            let hue = theme
                .get(ACCENT)
                .and_then(Rgba::parse)
                .map_or(0.0, |c| hsl(c).0);
            changed(0, hue);
        });
        *self.watching.lock().unwrap_or_else(PoisonError::into_inner) = Some(sub);
    }
}

#[cfg(test)]
mod tests {
    use super::hsl;
    use architect_theme::Rgba;

    #[test]
    fn a_colour_s_hue() {
        let h = |c: Rgba| hsl(c).0.round();
        assert_eq!(h(Rgba::rgb(1.0, 0.0, 0.0)), 0.0);
        assert_eq!(h(Rgba::rgb(0.0, 1.0, 0.0)), 120.0);
        assert_eq!(h(Rgba::rgb(0.0, 0.0, 1.0)), 240.0);
    }
}
