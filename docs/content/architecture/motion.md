+++
title = "Motion: one clock for every animation"
description = "Springs and coasts on one clock: the wall clock in a window, architect-guide's virtual clock in a film or a test. The workspace's docks, tab moves, stacked tabs and splits move on it."
weight = 55
+++

> **Status: landed (2026-11-01).** The primitive is `architect_shell::motion`
> (`features/shell/src/motion.rs`); the shell's drawers and sidebars and
> the workspace host's docks, tabs, stacked strips and splits move on it.
> Its springs were timed to Obsidian's, measured frame by frame (#155,
> [The reference: Obsidian](#the-reference-obsidian)).

Before this, the workspace had no animation: a sidebar opened in one
frame, a dropped tab jumped to its place, a split appeared at full size.
The only motion was the shell's phone drawer, eased by a `futures_timer`
loop on the wall clock, which `architect-guide`'s runner could not drive,
so a film never showed it.

## The primitive

A `Motion` is a value that moves (`features/shell/src/motion.rs`
`Motion`, made with `use_motion`):

- **sent** somewhere with `Motion::to` (`Motion::to_then` calls back
  when it gets there), it moves on a `Spring`: a response (how long one
  swing takes, seconds) and a damping (1 stops without passing its target,
  below 1 overshoots). The spring is solved, not stepped
  (`Spring::step`), so where it is depends on the time alone: a slow frame
  moves it further, never slower, and a film is the same at any frame rate;
- **held** with `Motion::hold` (a finger, a trackpad, a scrollbar's
  drag), it is where it is put, and it measures how fast it is moved;
- **let go** into `Motion::to` or `Motion::coast` it starts at that
  speed: a spring sent elsewhere mid-way, or a held value let go, keeps
  its velocity. A coast slows by `e` every `COAST` seconds and is sprung
  back inside its bounds if it runs past one.

Its state is plain data (`Track`), tested without a renderer. Its value is
a signal: the component that reads `Motion::value` redraws each frame it
moves; a `Motion` copied into another component's handler still runs its
frames in the component that made it.

The springs used, timed to Obsidian's (measured below), none
overshooting: `Spring::PANEL` (0.12 s: a sidebar, a dock, a split),
`Spring::SNAPPY` (0.22 s: tabs, a stacked strip's wheel notch and
spine), `Spring::TRACK` (0.11 s: the drop highlight following the
pointer). `Spring::SHEET` (0.32 s) is a phone's drawer, its tab switcher,
the keyboard's lift and a tour's spotlight
(`features/guide/src/tour.rs` `TourLayer`), which the study (on a desk)
did not measure.

## One clock

`features/shell/src/motion.rs` `now` and `next_frame` are the clock every
motion reads:

- **In a window** it is the wall clock, a frame each `FRAME` (60 fps),
  on boundaries of the clock so every motion moves in the same frames
  (one redraw for all of them).
- **In a run** (`features/guide/src/run.rs` `run`) the runner starts a
  `VirtualClock` on its thread before it opens the documents and moves it
  to frame *n*'s time (`n / fps`) each frame, then polls the documents and
  lays them out. Stylo's own transitions are resolved at the same time, so
  a film shows every in-between frame of both, the same on every run, and
  a test (no film) runs the same frames. While a motion waits for the next
  frame, waiting on a step's `done` is frames too, so a close that waits
  for its group to fold lands.

## Reduced motion

`set_reduced_motion` is the app's setting (`Some(true)` reduced, `None`
the OS's preference, read on macOS from `NSWorkspace`); `reduced_motion`
is what holds. Reduced, every `to` jumps to its end (and calls its
`to_then`), and every coast stops where it is. A virtual clock ignores the
OS, so a film is the same on every machine. The lab's Settings ›
Appearance › Reduce motion is the setting (`apps/lab/ui/src/lib.rs`).

## What moves

| What | How | Clipped or reflowed | Where |
|---|---|---|---|
| The desk's sidebars (the shell's left, its right side) | width on `Spring::PANEL`, 0 → the side's width | clipped: the side is laid out at its full width inside a box the motion sizes, held to the window's edge | `features/shell/src/shell/mod.rs` `AppShell` |
| A tablet's sidebar folding to its rail | width from the rail's to the side's | clipped, the rail shown once folded | `AppShell` |
| The phone drawer | its travel 0–1 on `Spring::SHEET`, held by the finger, let go at its speed | slides over the page | `AppShell` |
| A workspace dock opening, folding, folding to its rail of icons | width (height for the bottom) on `Spring::PANEL` | clipped: the tree keeps the dock's size, held to the window's edge; the rail shows once folded | `features/workspace/src/host/dock.rs` `DockView` |
| A tab dragged along its bar | the tab follows the pointer (held), the tabs it passes slide aside by its width on `Spring::SNAPPY` | `transform`s only: the bar is not laid out again | `features/workspace/src/host/slide.rs` |
| A tab dragged off its bar, over another | its place closes up; the other bar opens a gap where it would go | `transform`s | `slide.rs` `aside` |
| A tab let go | every tab of the bars it touches is moved back by how far its place moved, then sprung to nothing, so nothing jumps; a tab landing in another bar starts there from where it was drawn | `transform`s | `slide.rs` `landed`, `after` |
| A drag that ends nowhere (Escape, outside with nowhere to go) | every tab springs back | | `slide.rs` `cancel` |
| The drop highlight | fades in where a drop would land, follows the pointer from zone to zone on `Spring::TRACK`, fades out when the drag ends (over the split growing in under it) | | `features/workspace/src/host/overlay.rs` `DropHighlight` |
| A split made by a drop | the new group's share grows 0 → its share on `Spring::PANEL`, the others giving way | clipped: laid out at the size the drop lit, held to the drop's edge | `features/workspace/src/host/group.rs` `SplitView`, `features/workspace/src/host/mod.rs` `Host::move_pane` |
| A split's last tab closed from its ✕ | its share folds to 0, then the close lands | clipped at the size it had | `Host::close_from_tab` |
| A divider dragged | 1:1 with the pointer, no motion | | `overlay.rs` `on_move` |
| A phone's tab switcher opening, and closing into a card | the pane's box to its card and back on `Spring::SHEET` (`Switch::zoom`, 0–1): the card is the pane's own view, laid out at the pane's size and `scale`d; the other cards and the bar fade and slide in | `scale` and `left`/`top` of an absolutely placed card (a `translate` is in device pixels on iOS) | `features/workspace/src/host/switcher.rs` |
| A switcher card closed (its ✕, or swiped past half its width or flung) | it fades and shrinks, or flies off at its speed; the cards after it slide into its place, each card's place a pair of motions following its slot on `Spring::SNAPPY` | | `switcher.rs` `CardView`, `close_card`, `on_up` |
| A switcher card lifted (held still) and dragged | it follows the finger (held); the others make room; let go, it springs into its slot | | `switcher.rs` `drop_lifted` |
| The switcher's grid scrolled | held by the finger, past an end it gives half as far; let go, it coasts and springs back inside | | `switcher.rs` `on_move` |
| A stacked strip | a trackpad's pixels move it point for point (macOS sends its coasting as more of them); a wheel's notch (40 points; it was 1) eases there from where it was headed on `Spring::SNAPPY`; a spine pressed eases to its pane on it; a scrollbar flicked coasts | panes slide over the ones before them, each to its spine; a pane's shadow deepens as it slides over | `group.rs` `Stacked`, `features/workspace/src/layout.rs` `slides` |

Which close animates: only a tab's ✕ on the last tab of a group in a
split (a command or a menu row closes at once, so a script that closes
and then acts sees the close).

## What Blitz draws well

`transform` and `opacity` change nothing in the layout: the tabs move by
`transform` only. A clipped box's `width` lays out only that box's row
(the tree inside keeps its size). A stacked pane is placed by `left` in an
absolutely positioned strip. Blitz has no `requestAnimationFrame`; the
wall clock's `FRAME` wakes the motions, and `FTS_FPS_LOG=1` (the FTS Blitz
fork's `blitz-shell`) logs frames and their cost each second.

The FTS fork draws a document's own updates at most `FTS_MAX_FPS` times a
second (30 unless set), so meters and clocks do not each repaint the
window at every vsync, but draws a running CSS animation at the display's
rate. `MotionFrames` (`features/shell/src/motion.rs`) bridges the two:
while any motion runs it runs an invisible 1 px CSS animation, and it
stops it when they rest. The element stays mounted: a node made just
after it went drew under its animation's opacity (blank)
(`skills/architect/references/blitz-quirks.md`). `AppShell` mounts it, and so do the
workspace's host (outside a shell) and its pop-outs. Measured in the lab
on an M4 Mac mini: a sidebar fold went from 32 to 140 frames a second,
6.8 ms a frame on average, 12.5 ms at worst.

A motion's frames run in its window's root scope (a motion may be sent
from a component that is going, a tab dropped into another bar); they end
once the component that made it goes, and every method on a motion whose
component has gone does nothing.

## Measured in the lab

The lab (`apps/lab/desktop`) driven with real pointer and wheel events,
recorded at the display's capture rate (about 29 fps):

| Interaction | What was seen |
|---|---|
| The sidebar's button | the sidebar folds in about 0.25 s, ease-out, no overshoot (the edge moved 63, 62, 38, 26, 20, 14, 6 points a frame); its content stays laid out and is clipped at the moving edge; it opens the same way |
| A tab dragged along its bar | the tab follows the pointer, the tabs it passes slide under it to the other side; let go, it settles where the gap was |
| A tab dragged into another group's bar | its place in its own bar closes up; it lands after the other bar's tab |
| A tab dropped in a group's top band | the group splits, the new group growing in from the top |
| Stacked tabs, two wheel notches | the next pane slides part of the way over the one before it and stays there: a small scroll is a small move |

Those were filmed before the springs were timed to Obsidian (below).
The lab's films (`lab-media … --keep-frames`, 20 fps on the virtual
clock), their raw frames compared before and after the tuning, against
Obsidian's (about 29 fps):

| Film | Before (progress each 50 ms) | After | Obsidian |
|---|---|---|---|
| `workspace-docks-ribbon`, the dock folding and opening | 0.25, 0.59, 0.79, 0.91, 0.95, 0.99, 1: about 0.3 s | 0.74, 0.97, 1: at rest in 0.15 s | about 0.8 at 70 ms, 0.95 at 105 ms, at rest in about 0.14 s |
| `workspace-split-drag`, the split growing in under the highlight | three moving frames (0.15 s) | two (0.1 s) | at once |
| `tab-reorder` | | the same frames (it moves on `Spring::SNAPPY`, unchanged) | Obsidian's tabs do not make room |
| `stacked-tabs` | | the same frames (its scrolls are a trackpad's pixels, moved point for point, and the spine it presses is in view) | |

## Signs

A stacked strip takes a wheel's delta as Blitz scrolls a box: a delta to
the right brings the strip's start into view, so a swipe moves it the way
it moves every scrolled box. `architect-guide`'s `@scroll{dx}` is signed
the same (a negative `dx` brings what is to the right into view).

## In the films

`architect-guide`'s `@scroll` is a trackpad's flick (quick, coasting to a
stop) in pixel deltas, as macOS sends them. The lab's scenarios film the
motion: `apps/lab/scenarios/stacked-tabs.styx` (a small scroll, a long
one, a spine pressed), `apps/lab/scenarios/tab-reorder.styx` (the tabs
making room), `apps/lab/scenarios/workspace-docks-ribbon.styx` (the
sidebar folding and opening), `apps/lab/scenarios/workspace-split-drag.styx`
(the highlight following, the split growing in, the divider, the close
folding back).

## The reference: Obsidian

The behaviour follows Obsidian's (sidebars that slide and clip, stacked
panes that slide over each other to their spines) and VS Code's and
Zed's tab drags (tabs that make room). Obsidian is a reference for
behaviour and timing only.

### Obsidian, measured

Obsidian 1.14.5 on an M4 Mac mini (airlock), a 1400 × 900 window, driven
with real pointer, key and wheel events (cua-driver) and filmed at the
display's capture rate (about 29 fps, a frame each 35 ms), then measured
frame by frame from the raw frames (edges and glyphs tracked along pixel
rows). A curve is fitted to each interaction's frames with its start
free between the last still frame and the first moving one; at 29 fps a
motion under 35 ms is one frame, so "at once" means within a frame.

| Interaction | Duration | Easing (progress each frame) | Clipped or reflowed | Sticky | Overshoot |
|---|---|---|---|---|---|
| The left sidebar opening and closing (its button, 300 px) | about 0.14 s (4–5 frames) | ease-out, steep: opening 0.43, 0.76, 0.91, 0.99, 1; closing 0.60, 0.87, 0.97, 1. A critically damped spring of 0.12 s fits | the sidebar's content stays put, clipped at the moving edge; the main area is laid out again each frame (its centred text column glides by half the sidebar's width; lines are not rewrapped at this width) | the ribbon, the window's controls | none |
| The right sidebar opening and closing | the same, about 0.14 s | ease-out: opening 0.53, 0.80, 0.93, 0.99, 1; closing 0.60, 0.84, 0.94, 1 | its content moves with its inner (moving) edge and is clipped by the window's edge; the main area reflows as on the left | the ribbon, the title bar's buttons | none |
| A tab dragged along its bar | as long as the drag | the tabs do not move: a ghost of the tab (its icon and name) follows the pointer, and a 2 px bar marks where it would land, jumping from gap to gap | | the tab stays in its place (and current) until the drop | |
| That tab let go | at once (one frame) | the tab is in its new place in the next frame, nothing slides | | | none |
| A tab dropped into another pane's bar | 0.21 s | its old bar closes up at once; in the new bar it grows from no width to its own (the tabs after it and the + slide aside) while its name fades in: ease-in-out (0.04, 0.16, 0.41, …, 0.89, 1.0) | the bar is laid out again each frame | | none |
| A tab closed from its ✕ | 0.20 s | the reverse: its width to nothing, its name fading, ease-in-out (0.02, …, 0.41, 0.71, 0.92, 0.96, 1) | laid out each frame | | none |
| The drop highlight | about 0.1 s | it is the insertion bar's box, then morphs (place and size) to the zone under the pointer: 0.13, 0.69, 1 (an ease-out of about 80–110 ms, a 0.11 s spring); it goes at once on the drop | an overlay | | none |
| A split made by a drop (a tab on a pane's right half) | at once | the two panes are at their shares in the frame after the drop; nothing grows in | the panes are laid out at their new size | | none |
| A split's last tab closed | at once | the split is gone in the next frame; the other pane takes its room | laid out | | none |
| A divider dragged | as long as the drag | 1:1 with the pointer; let go, nothing settles | both panes reflow each frame (their text rewraps) | | none |
| Tabs stacked (the command) | about 0.75 s | the panes lay out stacked at once, then the strip scrolls to the current pane: a smooth scroll easing out (each frame 0.8 of the one before) | | the spines pile at both ends | none |
| A stacked strip, one wheel notch | 0.20 s | about 80 px a notch (as cua-driver sends one), smooth-scrolled: ease-in-out (0.13, 0.36, 0.66, 0.85, 0.99, 1); a 0.22 s critically damped spring is the nearest spring | a pane slides over the one before it | each pane stops at its spine; the spines pile at the left and the right | none |
| A stacked strip, three notches, and a flick (15 notches at once) | 0.2 s; about 0.6 s | the same smooth scroll, each notch added to where it was headed; a flick runs at about 2800 px/s and eases into its end | | the panes stop at their spines, also mid-flick | none |
| A spine clicked | about 0.9 s (here 3900 px, 5 panes) | a smooth scroll, ease-in-out with a long ease-out tail (its speed falling by about 8× in its last 0.3 s); the time grows with the distance | | the spines | none |
| A tab popped out (Move current tab to new window) | at once | the tab leaves its stack in one frame (the next takes its place), and a new window opens with no animation of its own | | | none |

What Obsidian does not do, and architect does: its tabs do not make room
during a drag and a dropped tab does not settle (architect follows VS
Code's and Zed's here, on `Spring::SNAPPY`, which is the timing of
Obsidian's own tab width changes); a new split does not grow in and a
closed one does not fold (architect grows and folds them on
`Spring::PANEL`, now as quick as Obsidian's sidebars, so they read as a
quick slide rather than a jump). Where they differ in layout: Obsidian's
right sidebar carries its content with its moving edge, architect's
keeps it held to the window's edge, as the left one does.

The constants tuned from it, in `features/shell/src/motion.rs`:
`Spring::PANEL` 0.32 → 0.12 s (the sidebars), `Spring::TRACK` 0.16 →
0.11 s (the highlight), `Spring::SNAPPY` kept at 0.22 s (it matched the
tabs' 0.20–0.21 s and the wheel's 0.20 s), and the stacked strip's
easing (`features/workspace/src/host/group.rs` `Stacked`) moved from
`Spring::PANEL` to `Spring::SNAPPY`. A spring takes the same time over
any distance, so a spine pressed far away moves in 0.25 s where
Obsidian's smooth scroll takes up to 0.9 s. `WHEEL_LINE` (40 points a
notch) is not changed: the 80 px measured is what cua-driver's notch
became in Chromium, not a hardware wheel's.
