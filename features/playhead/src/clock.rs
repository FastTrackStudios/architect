//! The seam between the deck and what is heard (#589).
//!
//! The deck keeps the time itself (a silent clock: arithmetic over
//! `Transport`). A [`Clock`] is what makes it heard: the web app's audio
//! element (`apps/web/src/app/audio.rs`), later a native player, or an
//! engine attached as the clock (studio#400, #528: designed here, not
//! built). A [`Follower`] tells the clock what the deck's active source
//! does (load its item's media, play, pause, seek), and hands what the
//! clock reads back to the deck ([`crate::deck::Deck::report`]): its time
//! while it really moves, its media's length, its end.
//!
//! The native apps run [`Silent`]: the bar and every view move, nothing is
//! heard (audible native playback is a later step).

use crate::deck::Deck;

/// What a clock read.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Reading {
    /// Seconds into its media.
    pub time: f64,
    /// Its media's length, once known.
    pub duration: Option<f64>,
    /// Whether it is really moving now (not paused, not waiting for data,
    /// not refused by the browser's autoplay rule): only then is its time
    /// the deck's.
    pub moving: bool,
    /// Whether it reached its media's end.
    pub ended: bool,
    /// Whether it cannot be heard (no stream for the media here, a file
    /// it cannot play, play refused): the deck's own clock goes on.
    pub failed: bool,
    /// Whether the app resolved the current item's media to a stream it
    /// plays: only then is `duration` the deck's (#589's media
    /// requirement: no length for a path it would not stream).
    pub resolved: bool,
}

/// What makes the deck heard.
pub trait Clock {
    /// `"media"` (an element the app owns), `"silent"`, later `"engine"`.
    fn kind(&self) -> &'static str;
    /// Item media at this vault path from now on (`None`: nothing to hear).
    fn load(&mut self, media: Option<&str>);
    /// Start.
    fn play(&mut self);
    /// Stop where it is.
    fn pause(&mut self);
    /// To `t` seconds.
    fn seek(&mut self, t: f64);
    /// What it reads now; `None` for a clock that only follows the deck.
    fn read(&self) -> Option<Reading>;
}

/// A clock nobody hears: the deck's own arithmetic is the time.
#[derive(Debug, Clone, Copy, Default)]
pub struct Silent;

impl Clock for Silent {
    fn kind(&self) -> &'static str {
        "silent"
    }
    fn load(&mut self, _: Option<&str>) {}
    fn play(&mut self) {}
    fn pause(&mut self) {}
    fn seek(&mut self, _: f64) {}
    fn read(&self) -> Option<Reading> {
        None
    }
}

/// Keeps a [`Clock`] on the deck's active source.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Follower {
    /// The source, item and media it has loaded.
    loaded: Option<(String, usize, Option<String>)>,
    epoch: u64,
    playing: bool,
}

impl Follower {
    /// Tells `clock` what changed on `deck` since the last time.
    pub fn follow(&mut self, deck: &Deck, clock: &mut dyn Clock, now: f64) {
        let Some(s) = deck.current() else {
            if self.loaded.take().is_some() {
                clock.pause();
                clock.load(None);
            }
            self.playing = false;
            return;
        };
        let media = s.item().and_then(|i| i.media.clone());
        let what = Some((s.id.clone(), s.index, media.clone()));
        if self.loaded != what {
            clock.load(media.as_deref());
            clock.seek(s.time(now));
            self.loaded = what;
            self.epoch = s.epoch;
            self.playing = !s.transport.playing;
        }
        if self.epoch != s.epoch {
            clock.seek(s.time(now));
            self.epoch = s.epoch;
        }
        if self.playing != s.transport.playing {
            if s.transport.playing {
                clock.play();
            } else {
                clock.pause();
            }
            self.playing = s.transport.playing;
        }
    }

    /// Hands what `clock` reads to `deck`: whether the views should
    /// hear of it.
    pub fn report(&self, deck: &mut Deck, clock: &dyn Clock, now: f64) -> bool {
        match clock.read() {
            Some(r) if self.loaded.is_some() => deck.report(&r, now),
            _ => false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::deck::{Caller, Control};
    use crate::model::Spec;

    #[derive(Default)]
    struct Fake {
        log: Vec<String>,
        reading: Option<Reading>,
    }

    impl Clock for Fake {
        fn kind(&self) -> &'static str {
            "media"
        }
        fn load(&mut self, media: Option<&str>) {
            self.log.push(format!("load {media:?}"));
        }
        fn play(&mut self) {
            self.log.push("play".into());
        }
        fn pause(&mut self) {
            self.log.push("pause".into());
        }
        fn seek(&mut self, t: f64) {
            self.log.push(format!("seek {t}"));
        }
        fn read(&self) -> Option<Reading> {
            self.reading
        }
    }

    #[test]
    fn a_clock_follows_the_active_source_and_reports_back() {
        let me = Caller::Extension("s".into());
        let mut d = Deck::new();
        d.audible = true;
        let set = Spec::parse(&json!({"queue": [
            {"id": "a", "title": "A", "media": "R/a.m4a"},
            {"id": "b", "title": "B", "media": "R/b.m4a"}
        ]}))
        .unwrap();
        d.apply(&me, Control::Set(set), None, 0.0).unwrap();
        let (mut f, mut c) = (Follower::default(), Fake::default());
        f.follow(&d, &mut c, 0.0);
        assert_eq!(c.log, ["load Some(\"R/a.m4a\")", "seek 0", "pause"]);
        c.log.clear();
        f.follow(&d, &mut c, 0.0);
        assert!(c.log.is_empty(), "nothing changed, nothing told");
        // No seek before its length is known.
        assert!(d.apply(&me, Control::Seek(7.0), None, 0.0).is_err());
        d.apply(&me, Control::Play, None, 0.0).unwrap();
        f.follow(&d, &mut c, 0.0);
        assert_eq!(c.log, ["play"]);
        c.log.clear();
        c.reading = Some(Reading {
            time: 1.0,
            duration: Some(60.0),
            moving: true,
            ended: false,
            failed: false,
            resolved: true,
        });
        assert!(f.report(&mut d, &c, 1_000.0));
        assert_eq!(d.state("s", 1_000.0).unwrap()["duration"], 60.0);
        d.apply(&me, Control::Seek(7.0), None, 1_000.0).unwrap();
        f.follow(&d, &mut c, 1_000.0);
        assert_eq!(c.log, ["seek 7"]);
        c.log.clear();
        d.apply(&me, Control::Next, None, 2_000.0).unwrap();
        f.follow(&d, &mut c, 2_000.0);
        assert_eq!(c.log, ["load Some(\"R/b.m4a\")", "seek 0", "play"]);
        c.log.clear();
        d.apply(
            &me,
            Control::Set(Spec::parse(&json!({"queue": []})).unwrap()),
            None,
            3_000.0,
        )
        .unwrap();
        f.follow(&d, &mut c, 3_000.0);
        assert_eq!(c.log, ["pause", "load None"]);
    }
}
