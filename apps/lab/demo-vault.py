"""Generate the lab's demo Obsidian vault: apps/lab/demo-vault/.

Run from the repo root (needs ImageMagick for the raster images and the PDF):

    nix shell nixpkgs#imagemagick -c python3 -I apps/lab/demo-vault.py

It deletes and rewrites apps/lab/demo-vault/ from scratch, deterministically:
the fixed reference date is 2026-11-05 (the real clock is never read) and all
randomness comes from seeded generators, so reruns produce identical files.

Git cannot keep empty directories, so the folders that must exist but hold
nothing (e.g. Archive/Empty) are listed one per line in
apps/lab/demo-vault/.empty-folders; the lab's seeding creates them from that
file when it copies the vault.
"""

import datetime as dt
import os
import random
import shutil
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.join(HERE, "demo-vault")
TODAY = dt.date(2026, 11, 5)
MTIME = dt.datetime(2026, 11, 5, 12, 0, 0, tzinfo=dt.timezone.utc).timestamp()
EMPTY_FOLDERS = ["Archive/Empty", "Inbox/Processed"]


class Block(list):
    """A list rendered as a YAML block list."""


def val(v):
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, list) and not isinstance(v, Block):
        return "[" + ", ".join(str(x) for x in v) + "]"
    if isinstance(v, str) and (v.startswith("[[") or ": " in v or v.startswith("#")):
        return '"' + v.replace('"', "'") + '"'
    return str(v)


def fm(d):
    out = ["---"]
    for k, v in d.items():
        if isinstance(v, Block):
            out.append(k + ":")
            out += ["  - " + str(x) for x in v]
        else:
            out.append(f"{k}: {val(v)}")
    out.append("---")
    return "\n".join(out) + "\n"


def W(path, props, body=""):
    full = os.path.join(ROOT, path)
    os.makedirs(os.path.dirname(full), exist_ok=True)
    text = (fm(props) if props else "") + body.strip("\n") + "\n"
    with open(full, "w", encoding="utf-8") as f:
        f.write(text)
    os.utime(full, (MTIME, MTIME))


def T(created, modified=None):
    d = {"created": created}
    if modified:
        d["modified"] = modified
    return d


def feat(name):
    return f"[[Attachments/{name}]]"


# ---------------------------------------------------------------- prose

SENT = [
    "The point of {t} is to make the next session easier than the last one.",
    "Slow, deliberate repetition beats fast sloppy repetition every time.",
    "Write down what felt hard, because tomorrow it will feel obvious and you will forget.",
    "A short daily habit around {t} outlasts any heroic weekend.",
    "It helps to record a take and listen back with fresh ears.",
    "Most mistakes with {t} come from rushing the setup, not from the main idea.",
    "Keep the notes small enough that you will actually reopen them.",
    "When progress stalls, change one variable and leave the rest alone.",
    "There is a difference between knowing about {t} and being able to do it on demand.",
    "Pair every new idea with an example you can play, run, cook or build in ten minutes.",
    "Review at the end of the week: what worked, what did not, what to drop.",
    "Constraints are a gift; a narrow brief makes {t} easier to start.",
    "Borrow structure from people who have done this longer, then adapt it.",
    "If a step needs willpower every time, redesign the step.",
    "Notice the small wins; they are the evidence that the system works.",
    "Tempo, tone and timing all matter, in roughly that order, when working on {t}.",
    "Link related notes so the connections appear when you need them, not when you remember them.",
    "Do not confuse a tidy note with an understood idea.",
    "Return to the basics of {t} whenever a new technique feels shaky.",
    "Ten focused minutes is a real session; count it.",
]


def prose(rng, topic, words):
    out, n = [], 0
    while n < words:
        para = " ".join(rng.choice(SENT).format(t=topic) for _ in range(rng.randint(4, 6)))
        out.append(para)
        n += len(para.split())
    return "\n\n".join(out)


def long_note(seed, topic, headings, words, lead=""):
    rng = random.Random(seed)
    per = words // len(headings)
    parts = [lead] if lead else []
    for h in headings:
        parts.append(f"## {h}\n\n" + prose(rng, topic, per))
    return "\n\n".join(parts)


# --------------------------------------------------------------- images


def mg(*args):
    subprocess.run(["magick", *args], check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)


def make_images():
    A = os.path.join(ROOT, "Attachments")
    os.makedirs(A, exist_ok=True)
    p = lambda n: os.path.join(A, n)
    mg("-size", "320x200", "gradient:#ff9a3c-#7a1fa2", p("gradient-sunrise.png"))
    mg("-size", "320x200", "gradient:#0b3d91-#33d6c9", p("gradient-ocean.png"))
    mg("-size", "320x200", "xc:#1b1f3a", "-fill", "#f4b942", "-draw", "circle 90,100 90,40",
       "-fill", "#e4572e", "-draw", "circle 180,110 180,60", "-fill", "#29b6a6",
       "-draw", "circle 250,80 250,45", p("shapes-circles.png"))
    mg("-size", "320x200", "xc:#f2efe6", "-fill", "#2d6a4f", "-draw", "rectangle 20,20 140,90",
       "-fill", "#d8572a", "-draw", "rectangle 160,60 300,180", "-fill", "#3a86ff",
       "-draw", "polygon 40,180 120,110 200,180", p("shapes-grid.png"))
    mg("-size", "360x120", "xc:#222831", "-fill", "#ffd369", "-gravity", "center", "-pointsize", "40",
       "-annotate", "0", "Architect Lab", p("label-lab.png"))
    mg("-seed", "11", "-size", "320x200", "plasma:fractal", p("plasma-one.png"))
    mg("-seed", "21", "-size", "320x200", "plasma:", "-quality", "82", p("plasma-a.jpg"))
    mg("-seed", "22", "-size", "320x200", "plasma:", "-quality", "82", p("plasma-b.jpg"))
    mg("-seed", "23", "-size", "320x200", "plasma:", "-quality", "82", p("plasma-c.jpg"))
    mg("-size", "320x200", "gradient:#2b1055-#f76b1c", "-quality", "85", p("gradient-dusk.jpg"))
    mg("-size", "320x200", "pattern:verticalsaw", "-colorspace", "sRGB", "-fill", "#e63946",
       "-tint", "70", "-quality", "85", p("stripes.jpg"))
    mg("-seed", "31", "-size", "320x200", "plasma:", "-blur", "0x4", "-swirl", "120", "-quality", "82",
       p("swirl.jpg"))
    mg("-size", "240x240", "radial-gradient:#ffe08a-#c1121f", "-quality", "85", p("sun.jpg"))
    svg = {
        "logo.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="200" height="200" viewBox="0 0 200 200">'
        '<rect width="200" height="200" rx="32" fill="#1b1f3a"/><path d="M40 150 L100 40 L160 150 Z" '
        'fill="none" stroke="#f4b942" stroke-width="12" stroke-linejoin="round"/>'
        '<circle cx="100" cy="115" r="14" fill="#29b6a6"/></svg>',
        "wave.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="320" height="120" viewBox="0 0 320 120">'
        '<rect width="320" height="120" fill="#eef6ff"/><path d="M0 60 Q40 0 80 60 T160 60 T240 60 T320 60" '
        'fill="none" stroke="#3a86ff" stroke-width="6"/></svg>',
        "badge.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="160" height="160" viewBox="0 0 160 160">'
        '<circle cx="80" cy="80" r="70" fill="#d8572a"/><circle cx="80" cy="80" r="50" fill="none" '
        'stroke="#fff" stroke-width="6" stroke-dasharray="10 8"/><text x="80" y="90" font-size="28" '
        'text-anchor="middle" fill="#fff" font-family="sans-serif">LAB</text></svg>',
        "diagram.svg": '<svg xmlns="http://www.w3.org/2000/svg" width="360" height="140" viewBox="0 0 360 140">'
        '<rect x="10" y="40" width="90" height="50" fill="#e0e7ff" stroke="#4f46e5"/>'
        '<rect x="135" y="40" width="90" height="50" fill="#dcfce7" stroke="#16a34a"/>'
        '<rect x="260" y="40" width="90" height="50" fill="#fee2e2" stroke="#dc2626"/>'
        '<path d="M100 65 H135 M225 65 H260" stroke="#333" stroke-width="3"/></svg>',
    }
    for n, s in svg.items():
        with open(p(n), "w") as f:
            f.write(s + "\n")
    # one-page PDF: an image plus a caption.
    mg(p("gradient-ocean.png"), "-background", "white", "-gravity", "south", "-splice", "0x40",
       "-fill", "#222", "-pointsize", "18", "-annotate", "+0+10", "Lab handout - demo vault",
       "-density", "72", p("Lab handout.pdf"))
    for n in os.listdir(A):
        os.utime(os.path.join(A, n), (MTIME, MTIME))


# ------------------------------------------------------------------ main

def main():
    if os.path.isdir(ROOT):
        shutil.rmtree(ROOT)
    os.makedirs(ROOT)
    make_images()
    home()
    music()
    health_home()
    work_learning()
    people_resources_archive()
    projects()
    inbox()
    journal()
    bases()
    scroll_test()
    originals()
    with open(os.path.join(ROOT, ".empty-folders"), "w") as f:
        f.write("\n".join(EMPTY_FOLDERS) + "\n")
    for e in EMPTY_FOLDERS:
        os.makedirs(os.path.join(ROOT, e), exist_ok=True)
    count = sum(len(fs) for _, _, fs in os.walk(ROOT))
    print(f"wrote {count} files to {ROOT}")


def home():
    W("Welcome.md", {
        "title": "Welcome", "tags": ["lab/demo", "start"], "status": "active",
        "feature": feat("label-lab.png"), **T("2026-09-01T08:00:00", "2026-11-05T09:00:00"),
    }, """
# Welcome

This is the lab's **demo vault**: a realistic Obsidian-style vault generated by
`apps/lab/demo-vault.py`. It has nested folders, every property type, tags,
tasks, embeds, images, Bases and a pile of journal notes. #lab/demo

![[label-lab.png]]

## Areas

- [[Music hub|Music]]: guitar technique, theory and songs
- [[Health hub|Health]]: running and sleep
- [[Work hub|Work]]: meetings and docs
- [[Learning hub|Learning]]: Rust and books
- [[Home hub|Home]]: garden, recipes, budget

## Elsewhere

- Projects: [[Projects/Album/project|Album]], [[Website Redesign]], [[Half Marathon]], [[Kitchen Remodel]]
- Journal: [[2026-11-05]], [[2026-W45]], [[2026-11]], [[2026-Q4]], [[2026]]
- Triage the [[Inbox/Quick thought about tempo|inbox]]
- Bases: see the `Bases/` folder, e.g. [[Bases/Projects.base|Projects]]
- Archive has a 300-note `Scroll test` folder, and the original lab samples
  live in [[Notes/The app shell]] and [[Songs/Amazing Grace]].

> [!tip] Unresolved links
> [[Trip to Lisbon]] and [[Mastering chain]] do not exist on purpose.
""")
    hubs = {
        "Music hub": ("Music", ["[[Alternate picking]]", "[[Fingerstyle patterns]]", "[[Barre chords]]",
                                "[[Circle of fifths]]", "[[Modes]]", "[[Voice leading]]", "[[Blue Moon Walk]]"]),
        "Health hub": ("Health", ["[[Training plan]]", "[[Race log]]", "[[Sleep]]"]),
        "Work hub": ("Work", ["[[Standup 2026-10-27]]", "[[Sprint planning]]", "[[Retro Q3]]",
                              "[[Architecture overview]]", "[[Onboarding checklist]]"]),
        "Learning hub": ("Learning", ["[[Ownership]]", "[[Traits]]", "[[Async]]", "[[Dune]]", "[[Deep Work]]",
                                      "[[Reading list]]"]),
        "Home hub": ("Home", ["[[Garden]]", "[[Sourdough]]", "[[Lentil soup]]", "[[Budget 2026]]"]),
    }
    for name, (area, links) in hubs.items():
        W(f"Areas/{name}.md", {"title": area, "type": "hub", "tags": ["hub"], "aliases": [area + " area"]},
          f"# {area}\n\nBack to [[Welcome]].\n\n" + "\n".join("- " + l for l in links))


def music():
    base = "Areas/Music"
    W(f"{base}/Guitar/Technique/Alternate picking.md", {
        "title": "Alternate picking", "tags": ["music/guitar", "practice"], "status": "active",
        "difficulty": 3, "bpm_goal": 140, **T("2026-08-12T19:00:00", "2026-10-22T20:15:00"),
    }, """
# Alternate picking

Strict down-up motion, even when skipping strings. Start at 80 bpm.

- Keep the wrist loose, #music/guitar
- Metronome on 2 and 4
- See [[Fingerstyle patterns]] for the other hand and [[Pedalboard]] for tone

- [x] Chromatic 1234 on every string
- [x] Three-note-per-string scales
- [ ] Reach 140 bpm cleanly
""")
    W(f"{base}/Guitar/Technique/Fingerstyle patterns.md", {
        "title": "Fingerstyle patterns", "tags": Block(["music/guitar", "practice"]), "status": "active",
        "feature": feat("shapes-circles.png"), "difficulty": 4, **T("2026-07-02T18:30:00", "2026-10-30T21:05:00"),
    }, """
# Fingerstyle patterns

![[shapes-circles.png]]

Travis pattern: thumb alternates bass, fingers take the top strings.

1. P-i-P-m
2. P-i-m-i
3. Arpeggio p-i-m-a

Apply it to [[Amazing Grace]] and [[Slow River]]. #music/guitar #music/songs
""")
    W(f"{base}/Guitar/Technique/Barre chords.md", {
        "title": "Barre chords", "tags": ["music/guitar"], "status": "stalled", "difficulty": 5,
        "created": "2026-06-20T17:45:00",
    }, """
# Barre chords

F major is the wall. Roll the index finger slightly toward its bone edge.
Related: [[Circle of fifths]]. Unwritten: [[Barre chord warmups]].
""")
    W(f"{base}/Guitar/Technique/Drafts.md", {"title": "Technique drafts", "tags": ["draft"]}, "Stub.")
    W(f"{base}/Guitar/Gear/Pedalboard.md", {
        "title": "Pedalboard", "tags": ["music/gear"], "status": "active", "feature": feat("shapes-grid.png"),
        "cost": 349.5, "owned": True, "aliases": ["Board", "Pedals"],
    }, """
# Pedalboard

![[shapes-grid.png]]

Order: tuner, compressor, overdrive, delay, reverb.

- [x] Buy patch cables
- [x] Velcro everything down
- [ ] Power supply with isolated outputs
- [ ] [[Pedal ideas]]
""")
    W(f"{base}/Guitar/Gear/Strings.md", {"title": "Strings", "tags": ["music/gear"], "owned": False,
                                         "last_changed": "2026-09-14"}, "Change every 6 weeks. 10-46 gauge. #music/gear")
    W(f"{base}/Theory/Circle of fifths.md", {
        "title": "Circle of fifths", "tags": ["music/theory"], "status": "done", "feature": feat("sun.jpg"),
        "aliases": ["Cycle of fifths"], **T("2026-05-03T09:00:00", "2026-09-01T10:00:00"),
    }, """
# Circle of fifths

![[sun.jpg]]

C G D A E B F# ... clockwise adds a sharp. Counter-clockwise adds a flat.
Used by [[Modes]] and [[Voice leading]]. #music/theory
""")
    W(f"{base}/Theory/Voice leading.md", {"title": "Voice leading", "tags": ["music/theory"], "status": "draft",
                                          "related": "[[Circle of fifths]]"},
      "# Voice leading\n\nMove each voice the smallest distance. In [[Blue Moon Walk]] the bass walks G-F#-E.\n")
    W(f"{base}/Theory/Modes.md", {
        "title": "Modes", "tags": ["music/theory", "long-read"], "status": "active",
        "feature": feat("plasma-a.jpg"), "difficulty": 4, "reviewed": "2026-10-15",
        **T("2026-04-18T12:00:00", "2026-10-15T08:45:00"),
    }, "# Modes\n\n![[plasma-a.jpg]]\n\nSeven modes of the major scale. Start from [[Circle of fifths]].\n\n"
       "![[Voice leading]]\n\n" + long_note(
        101, "modal playing",
        ["Ionian", "Dorian", "Phrygian", "Lydian", "Mixolydian", "Aeolian", "Locrian", "Choosing a mode",
         "Practice routine", "Listening list"], 1500))
    W("Songs/Blue Moon Walk.md", {
        "title": "Blue Moon Walk", "type": "song", "artist": "Cody Wright", "key": "G", "tempo": 96,
        "tags": ["music/songs", "original"], "feature": feat("gradient-dusk.jpg"), "status": "writing",
        "album": "[[Projects/Album/project|Album]]", **T("2026-09-30T22:10:00", "2026-11-03T23:40:00"),
    }, """
# Blue Moon Walk

![[gradient-dusk.jpg]]

## Chords

```kf
[Verse]
| G | G7 | C | G |
| G | Em | D | D7 |

[Chorus]
| C | G | D | G |
```

## Notes

Walking bass in the verse, see [[Voice leading]]. Mix notes live in [[Mixing]].
- [x] Write verse
- [x] Write chorus
- [ ] Bridge
""")
    W("Songs/Slow River.md", {
        "title": "Slow River", "type": "song", "artist": "Traditional", "key": "D", "tempo": 68,
        "tags": ["music/songs"], "status": "learning", "performed": False,
    }, """
# Slow River

```kf
[Intro]
| D | A/C# | Bm | G |
[Verse]
| D | A | Bm | G | D | A | G | D |
```

Fingerstyle: [[Fingerstyle patterns]].
""")


def health_home():
    W("Areas/Health/Fitness/Running/Training plan.md", {
        "title": "Training plan", "tags": ["health/running"], "status": "active", "week": 6,
        "project": "[[Half Marathon]]", "days": Block(["Tue intervals", "Thu tempo", "Sun long"]),
    }, """
# Training plan

Target: [[Half Marathon]] on 2026-12-06.

- [x] Week 1 base miles
- [x] Week 2 base miles
- [x] Week 3 hills
- [ ] Week 7 long run 16 km
- [ ] Taper
""")
    W("Areas/Health/Fitness/Running/Race log.md", {
        "title": "Race log", "tags": ["health/running"], "races": Block(["10k 2025-05-11", "5k 2026-03-02"]),
        "best_5k": "21:40",
    }, "# Race log\n\n| Date | Distance | Time |\n|---|---|---|\n| 2025-05-11 | 10k | 47:12 |\n| 2026-03-02 | 5k | 21:40 |\n")
    W("Areas/Health/Sleep.md", {"title": "Sleep", "tags": ["health"], "avg_hours": 7.4, "tracking": True},
      "# Sleep\n\nTarget 7.5 hours. Screens off at 22:00. See [[2026-W44]].\n")
    W("Areas/Home/Garden.md", {
        "title": "Garden", "tags": ["home", "garden"], "status": "active", "feature": feat("swirl.jpg"),
        "plants": ["tomato", "basil", "sage"], "planted": "2026-04-25",
    }, """
# Garden

![[swirl.jpg]]

- [x] Winterize the beds
- [x] Collect seeds
- [x] Order bulbs

All done for the season. Compost notes: [[Sourdough]] discard goes in, too.
""")
    W("Areas/Home/Recipes/Sourdough.md", {
        "title": "Sourdough", "tags": ["home/recipes", "food"], "feature": feat("stripes.jpg"),
        "hydration": 75, "servings": 2, "rating": 5, "last_baked": "2026-11-01",
    }, """
# Sourdough

![[stripes.jpg]]

500 g flour, 375 g water, 100 g starter, 10 g salt. Fold every 30 minutes
for 2 hours. Cold proof overnight. See [[Lentil soup]] for dinner. #home/recipes
""")
    W("Areas/Home/Recipes/Lentil soup.md", {
        "title": "Lentil soup", "tags": ["home/recipes", "food"], "servings": 4, "vegetarian": True,
        "rating": 4,
    }, "# Lentil soup\n\nLentils, carrot, onion, cumin, lemon. 35 minutes.\n")
    W("Areas/Home/Finance/Budget 2026.md", {
        "title": "Budget 2026", "tags": ["finance"], "feature": feat("wave.svg"), "monthly_income": 4200,
        "reviewed": "2026-11-01T20:00:00", "accounts": Block(["checking", "savings", "brokerage"]),
    }, """
# Budget 2026

![[wave.svg]]

| Category | Monthly |
|---|---|
| Rent | 1450 |
| Food | 520 |
| Gear | 150 |

- [x] Review Q3
- [ ] Set Q4 gear cap ([[Pedalboard]])
""")


def work_learning():
    W("Areas/Work/Meetings/2026/Standup 2026-10-27.md", {
        "title": "Standup 2026-10-27", "type": "meeting", "tags": ["work/meeting"], "date": "2026-10-27",
        "attendees": ["Alex Rivera", "Sam Okafor"], **T("2026-10-27T09:00:00", "2026-10-27T09:20:00"),
    }, """
# Standup 2026-10-27

#work/meeting with [[Alex Rivera]] and [[Sam Okafor]].

- [x] Unblock the CI runner
- [ ] Review PR for [[Website Redesign]]
""")
    W("Areas/Work/Meetings/2026/Sprint planning.md", {
        "title": "Sprint planning", "type": "meeting", "tags": ["work/meeting", "planning"], "date": "2026-11-02",
        "attendees": Block(["Alex Rivera", "Sam Okafor", "Cody"]),
    }, """
# Sprint planning

Scope for the sprint, tied to [[Architecture overview]].

- [x] Estimate stories
- [x] Pick sprint goal
- [x] Assign owners
""")
    W("Areas/Work/Meetings/2026/Retro Q3.md", {
        "title": "Retro Q3", "type": "meeting", "tags": ["work/meeting", "retro"], "date": "2026-10-02",
        "outcome": "Ship smaller PRs",
    }, "# Retro Q3\n\nWent well: releases. Not well: review latency. Follow-up in [[2026-Q4]]. [[Q1 planning]] is later.\n")
    W("Areas/Work/Docs/Architecture overview.md", {
        "title": "Architecture overview", "tags": ["work/docs", "long-read"], "status": "active",
        "feature": feat("diagram.svg"), "owner": "[[Alex Rivera]]", **T("2026-03-10T10:00:00", "2026-10-29T16:30:00"),
    }, "# Architecture overview\n\n![[diagram.svg]]\n\nThree layers: client, sync and store.\n\n"
       "![[Onboarding checklist]]\n\n" + long_note(
        202, "the architecture",
        ["Client", "Sync", "Store", "Auth", "Observability", "Failure modes", "Testing", "Release process",
         "Open questions"], 1500))
    W("Areas/Work/Docs/Onboarding checklist.md", {
        "title": "Onboarding checklist", "tags": ["work/docs"], "status": "done",
    }, """
# Onboarding checklist

- [x] Accounts
- [x] Dev shell
- [x] First PR
- [x] Meet [[Alex Rivera]]
""")
    W("Areas/Learning/Rust/Ownership.md", {
        "title": "Ownership", "tags": ["learning/rust"], "status": "done", "rating": 5,
    }, "# Ownership\n\nEach value has one owner; borrow with `&` or `&mut`. Next: [[Traits]].\n")
    W("Areas/Learning/Rust/Traits.md", {
        "title": "Traits", "tags": ["learning/rust"], "status": "active", "rating": 4,
    }, """
# Traits

```rust
trait Chart { fn render(&self) -> String; }
```

Then [[Async]].
""")
    W("Areas/Learning/Rust/Async.md", {"title": "Async", "tags": ["learning/rust"], "status": "draft"},
      "# Async\n\nFutures are inert until polled.\n")
    W("Areas/Learning/Books/Dune.md", {
        "title": "Dune", "type": "book", "tags": ["reading/fiction"], "author": "Frank Herbert",
        "pages": 612, "finished": "2026-09-12", "rating": 5, "feature": feat("sun.jpg"),
    }, "# Dune\n\n![[sun.jpg]]\n\nThe spice must flow. Listed in [[Reading list]].\n")
    W("Areas/Learning/Books/Deep Work.md", {
        "title": "Deep Work", "type": "book", "tags": ["reading/nonfiction"], "author": "Cal Newport",
        "pages": 296, "finished": False, "rating": 4,
    }, "# Deep Work\n\nBlock time. Shut notifications. Notes inform [[Sleep]] and [[2026-W43]].\n")
    W("Areas/Learning/Books/Reading list.md", {"title": "Reading list", "tags": ["reading"]}, """
# Reading list

- [x] [[Dune]]
- [ ] [[Deep Work]]
- [ ] Gödel, Escher, Bach
- [ ] The Pragmatic Programmer
""")


def people_resources_archive():
    W("Areas/People/Alex Rivera.md", {
        "title": "Alex Rivera", "type": "person", "tags": ["people", "work"], "aliases": ["Alex", "AR"],
        "company": "Fasttrack", "email": "alex@example.com", "birthday": "1990-03-14",
    }, "# Alex Rivera\n\nTech lead. Owner of [[Architecture overview]].\n")
    W("Areas/People/Sam Okafor.md", {
        "title": "Sam Okafor", "type": "person", "tags": ["people", "work"], "aliases": ["Sam"],
        "company": "Fasttrack",
    }, "# Sam Okafor\n\nDesigner on [[Website Redesign]].\n")
    W("Resources/Ideas.md", {"title": "Ideas", "tags": ["ideas"], "status": "inbox"},
      "# Ideas\n\n- A metronome that listens\n- [[Trip to Lisbon]] planning note\n- Tape-style delay plugin\n")
    W("Resources/Snippets/Shell one-liners.md", {"title": "Shell one-liners", "tags": ["dev/snippets"]},
      "# Shell one-liners\n\n```sh\ngit log --first-parent --oneline | head\n```\n")
    W("Resources/Snippets/Regex.md", {"title": "Regex", "tags": ["dev/snippets"]}, "Tiny. `^\\d{4}-\\d{2}-\\d{2}$`")
    W("Archive/Old ideas.md", {"title": "Old ideas", "tags": ["archive"], "status": "archived"},
      "# Old ideas\n\nSuperseded by [[Ideas]].\n")
    W("Archive/2025/Retired project.md", {
        "title": "Retired project", "type": "project", "status": "cancelled", "due": "2025-12-31",
        "area": "Work", "tags": ["archive"],
    }, "# Retired project\n\nCancelled in favour of [[Website Redesign]].\n")
    W("Archive/2025/Mixtape.md", {"title": "Mixtape", "tags": ["archive", "music"]}, "Gone but linked from [[2025]].")


def projects():
    P = "Projects"
    W(f"{P}/Album/project.md", {
        "title": "Album", "type": "project", "status": "active", "due": "2027-03-01", "area": "Music",
        "tags": ["project", "music/songs"], "feature": feat("plasma-b.jpg"), "progress": 40,
        **T("2026-06-01T10:00:00", "2026-11-04T22:00:00"),
    }, """
# Album

![[plasma-b.jpg]]

Ten songs. Notes: [[Tracklist]], [[Studio session notes]]. Sub-project: [[Mixing]].

- [x] Demo all songs
- [ ] Record guitars
- [ ] Mastering chain ([[Mastering chain]])
""")
    W(f"{P}/Album/Tracklist.md", {"title": "Tracklist", "tags": ["music/songs"], "project": "[[Projects/Album/project|Album]]"},
      "# Tracklist\n\n1. [[Blue Moon Walk]]\n2. [[Slow River]]\n3. [[Amazing Grace]]\n\nBack to [[Projects/Album/project|Album]].\n")
    W(f"{P}/Album/Studio session notes.md", {
        "title": "Studio session notes", "tags": ["music/recording"], "session_start": "2026-10-18T10:00:00",
        "project": "[[Projects/Album/project|Album]]",
    }, "# Studio session notes\n\n![[plasma-c.jpg]]\n\nTracked drums. Room mics sounded great. Part of [[Projects/Album/project|Album]].\n")
    W(f"{P}/Album/Mixing/project.md", {
        "title": "Mixing", "type": "project", "status": "planned", "due": "2027-01-15", "area": "Music",
        "parent": "[[Projects/Album/project|Album]]", "tags": ["project", "music/mixing"],
        "feature": feat("swirl.jpg"),
    }, """
# Mixing

Sub-project of [[Projects/Album/project|Album]].

![[swirl.jpg]]

- [ ] Gain staging pass
- [ ] Balance drums and bass
- [ ] Reference against [[Reference tracks]]
""")
    W(f"{P}/Album/Mixing/Reference tracks.md", {"title": "Reference tracks", "tags": ["music/mixing"]},
      "# Reference tracks\n\nThree favourites. Used by [[Mixing]].\n")
    W(f"{P}/Album/Mixing/Mix notes.md", {"title": "Mix notes", "tags": ["music/mixing"]},
      "# Mix notes\n\nKick: cut 300 Hz. Part of [[Mixing]].\n")
    W(f"{P}/Website Redesign/project.md", {
        "title": "Website Redesign", "type": "project", "status": "active", "due": "2026-12-15", "area": "Work",
        "tags": ["project", "work"], "feature": feat("gradient-ocean.png"), "owner": "[[Sam Okafor]]",
        "progress": 65,
    }, """
# Website Redesign

![[gradient-ocean.png]]

See [[Sitemap]] and [[Copy deck]].

- [x] Audit current site
- [x] Wireframes
- [ ] Visual design
- [ ] Launch
""")
    W(f"{P}/Website Redesign/Sitemap.md", {"title": "Sitemap", "tags": ["work"]},
      "# Sitemap\n\nHome, Music, Shop, About. Project: [[Website Redesign]].\n")
    W(f"{P}/Website Redesign/Copy deck.md", {"title": "Copy deck", "tags": ["work"], "words": 1840},
      "# Copy deck\n\nHeadlines first. Project: [[Website Redesign]].\n")
    W(f"{P}/Half Marathon/project.md", {
        "title": "Half Marathon", "type": "project", "status": "active", "due": "2026-12-06", "area": "Health",
        "tags": ["project", "health/running"], "feature": feat("gradient-sunrise.png"), "progress": 55,
    }, "# Half Marathon\n\n![[gradient-sunrise.png]]\n\nPlan in [[Training plan]]; log in [[Training log]].\n")
    W(f"{P}/Half Marathon/Training log.md", {"title": "Training log", "tags": ["health/running"],
                                              "total_km": 212.5},
      "# Training log\n\nWeek 6: 38 km. Project: [[Half Marathon]].\n")
    W(f"{P}/Half Marathon/Gear.md", {"title": "Gear", "tags": ["health/running"]},
      "- [x] Shoes\n- [ ] Gels\n\nFor [[Half Marathon]].\n")
    W(f"{P}/Kitchen Remodel/project.md", {
        "title": "Kitchen Remodel", "type": "project", "status": "on-hold", "due": "2027-06-30", "area": "Home",
        "tags": ["project", "home"], "feature": feat("shapes-grid.png"), "budget": 18000,
    }, "# Kitchen Remodel\n\n![[shapes-grid.png]]\n\nOn hold until spring. [[Quotes]], [[Shopping list]].\n")
    W(f"{P}/Kitchen Remodel/Quotes.md", {"title": "Quotes", "tags": ["home"], "received": "2026-10-20T14:00:00"},
      "# Quotes\n\nThree contractors. Cheapest is not best. For [[Kitchen Remodel]].\n")
    W(f"{P}/Kitchen Remodel/Shopping list.md", {"title": "Shopping list", "tags": ["home"]},
      "# Shopping list\n\n- [ ] Faucet\n- [ ] Cabinet pulls\n- [x] Tape measure\n\nFor [[Kitchen Remodel]].\n")


def inbox():
    caps = [
        ("Quick thought about tempo", "quick-add", "2026-11-04T08:12:00", "Try the bridge of [[Blue Moon Walk]] at 88 bpm."),
        ("Article on spaced repetition", "share", "2026-11-03T19:40:00", "https://example.com/spaced-repetition\n\nRead later."),
        ("Re: invoice question", "email", "2026-11-02T11:05:00", "Forward to accounting. #finance"),
        ("Gift idea", "quick-add", "2026-11-01T17:30:00", "Capo + strap for the guitarist in the family."),
        ("Voice memo: chorus melody", "share", "2026-10-31T23:15:00", "Hum: da da DUM. Add to [[Slow River]]?"),
    ]
    for t, s, c, b in caps:
        W(f"Inbox/{t}.md", {"title": t, "type": "capture", "captured": c, "source": s, "processed": False},
          f"{b}\n")


def journal():
    rng = random.Random(7)
    lines = [
        "Slow morning, coffee, then an hour on the guitar.", "Long run in the rain; legs felt heavy.",
        "Deep work block went well, shipped a small PR.", "Baked a loaf, crumb was open and glossy.",
        "Studio day: tracked rhythm guitars.", "Rest day. Read twenty pages.",
        "Meetings all afternoon; evening walk helped.", "Tried a new fingerstyle pattern, still clumsy.",
        "Budget review took longer than expected.", "Quiet day, cleaned the desk and filed notes.",
    ]
    tasks = ["Email the contractor", "Practice scales 20 min", "Water the plants", "Review PR",
             "Back up the session", "Stretch", "Plan tomorrow", "Update [[Training plan]]"]
    for i in range(30):
        d = TODAY - dt.timedelta(days=29 - i)
        iso = d.isocalendar()
        wk = f"{iso[0]}-W{iso[1]:02d}"
        nt = rng.randint(0, 3)
        chosen = rng.sample(tasks, nt)
        body = [f"# {d.strftime('%A, %Y-%m-%d')}", "",
                f"Week [[{wk}]] | Month [[{d.strftime('%Y-%m')}]]", "", rng.choice(lines)]
        if rng.random() < 0.4:
            body += ["", rng.choice(lines)]
        if chosen:
            body += ["", "## Tasks"]
            body += [f"- [{'x' if (d < TODAY and rng.random() < 0.7) else ' '}] {t}" for t in chosen]
        if i % 7 == 3:
            body += ["", "#journal/highlight"]
        props = {"type": "daily", "date": d.isoformat(), "mood": rng.randint(2, 5), "tags": ["daily"],
                 "exercised": rng.random() < 0.5, "created": f"{d.isoformat()}T07:{rng.randint(10, 59)}:00"}
        W(f"Journal/Daily/{d.isoformat()}.md", props, "\n".join(body))
    for w in range(40, 46):
        mon = dt.date.fromisocalendar(2026, w, 1)
        sun = dt.date.fromisocalendar(2026, w, 7)
        W(f"Journal/Weekly/2026-W{w}.md", {"type": "weekly", "week": w, "start": mon.isoformat(),
                                           "end": sun.isoformat(), "tags": ["weekly"]},
          f"# 2026-W{w}\n\n{mon} to {sun}. Month: [[{mon.strftime('%Y-%m')}]].\n\n"
          f"{rng.choice(lines)}\n\n- [{'x' if w < 45 else ' '}] Weekly review\n- [ ] Plan next week\n")
    mon_notes = {8: "August: summer, started the album demos.", 9: "September: finished Dune, vault set up.",
                 10: "October: studio sessions and training ramp.", 11: "November: mixing plans and the half marathon taper."}
    for m, t in mon_notes.items():
        q = (m - 1) // 3 + 1
        W(f"Journal/Monthly/2026-{m:02d}.md", {"type": "monthly", "month": f"2026-{m:02d}", "tags": ["monthly"]},
          f"# 2026-{m:02d}\n\n{t}\n\nQuarter: [[2026-Q{q}]].\n\n- [x] Month review\n")
    W("Journal/Quarterly/2026-Q3.md", {"type": "quarterly", "quarter": "2026-Q3", "tags": ["quarterly"]},
      "# 2026-Q3\n\nMonths: [[2026-08]], [[2026-09]]. Retro: [[Retro Q3]].\n\n- [x] Quarter review\n")
    W("Journal/Quarterly/2026-Q4.md", {"type": "quarterly", "quarter": "2026-Q4", "tags": ["quarterly"]},
      "# 2026-Q4\n\nMonths: [[2026-10]], [[2026-11]].\n\nGoals: ship [[Website Redesign]], run [[Half Marathon]].\n\n- [ ] Review\n")
    W("Journal/Yearly/2025.md", {"type": "yearly", "year": 2025, "tags": ["yearly"]},
      "# 2025\n\nA quieter year. Archive: [[Mixtape]].\n")
    W("Journal/Yearly/2026.md", {"type": "yearly", "year": 2026, "tags": ["yearly"]},
      "# 2026\n\nQuarters: [[2026-Q3]], [[2026-Q4]]. Themes: songs, running, the vault.\n\n- [ ] Year review\n")


def bases():
    W("Bases/Projects.base", None, """
filters:
  and:
    - 'type == "project"'
views:
  - type: table
    name: All projects
    order:
      - file.name
      - status
      - due
      - area
    sort:
      - property: due
        direction: ASC
""")
    W("Bases/Songs.base", None, """
filters:
  and:
    - 'type == "song"'
views:
  - type: cards
    name: Songs
    image: note.feature
    order:
      - file.name
      - artist
      - key
      - tempo
""")
    W("Bases/Notes by status.base", None, """
filters:
  and:
    - 'status != null'
views:
  - type: table
    name: By status
    order:
      - file.name
      - status
    groupBy:
      property: status
      direction: ASC
""")
    W("Bases/Music notes.base", None, """
filters:
  or:
    - file.hasTag("music")
    - file.inFolder("Areas/Music")
views:
  - type: list
    name: Music
    order:
      - file.name
      - status
""")
    W("Bases/Inbox.base", None, """
filters:
  and:
    - file.inFolder("Inbox")
    - 'type == "capture"'
views:
  - type: table
    name: By source
    order:
      - file.name
      - captured
      - source
    groupBy:
      property: source
      direction: ASC
""")


def scroll_test():
    rng = random.Random(3)
    tags = ["scroll", "scroll/a", "scroll/b"]
    for i in range(1, 301):
        n = f"Note {i:03d}"
        W(f"Archive/Scroll test/{n}.md", {"title": n, "tags": [rng.choice(tags)], "index": i},
          f"# {n}\n\n{rng.choice(SENT).format(t='scrolling')}\n")


def originals():
    """The lab's original sample vault (apps/lab/vault), kept verbatim."""
    W("Notes/Lab sample welcome.md", {"title": "Welcome", "tags": ["lab", "start"], "status": "active"}, """
# Welcome

This folder is the lab's vault: notes with properties (the frontmatter
above), links between them and tags. Open [[The app shell]] next, or read
how [[Notes/Actions everywhere|actions]] reach every surface.

The songs live in [[Songs/Amazing Grace]]. #lab/sample
""")
    W("Notes/Actions everywhere.md", {"title": "Actions everywhere", "tags": ["actions"], "status": "done"},
      "Every action is one entry on the action list: ⌘K, the menu, the control\n"
      "socket and MCP all run it. Try `VAULT_READ {\"path\": \"Welcome.md\"}`. #agents")
    W("Notes/The app shell.md", {"title": "The app shell", "tags": ["shell"], "status": "draft"},
      "The shell holds the tabs, the sidebar and the toolbars. See [[Welcome]]\nand [[The workspace]].")
    W("Notes/The workspace.md", {"title": "The workspace", "tags": ["shell", "workspace"], "status": "active"},
      "Panes, docks and screensets. Back to [[The app shell]]. A link to\n[[A note nobody wrote]] resolves to nothing.")
    W("Songs/Amazing Grace.md", {"title": "Amazing Grace", "key": "G", "bpm": 72, "tags": ["song"]},
      "A hymn, in [[Welcome]]'s sample. Verse one in G.")


if __name__ == "__main__":
    sys.exit(main())
