+++
title = "Motion: one clock for every animation"
description = "Springs and coasts on one clock: the wall clock in a window, architect-guide's virtual clock in a film or a test. The workspace's docks, tab moves, stacked tabs and splits move on it."
weight = 55
+++

> **Status: landed (2026-11-01).** The primitive is `architect_shell::motion`
> (`features/shell/src/motion.rs`); the shell's drawers and sidebars and
> the workspace host's docks, tabs, stacked strips and splits move on it.

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

The springs used: `Spring::PANEL` (0.32 s, no overshoot: a sidebar, a
dock, a split, a stacked strip), `Spring::SNAPPY` (0.22 s: tabs),
`Spring::TRACK` (0.16 s: the drop highlight following the pointer).

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
| The phone drawer | its travel 0–1, held by the finger, let go at its speed | slides over the page | `AppShell` |
| A workspace dock opening, folding, folding to its rail of icons | width (height for the bottom) on `Spring::PANEL` | clipped: the tree keeps the dock's size, held to the window's edge; the rail shows once folded | `features/workspace/src/host/dock.rs` `DockView` |
| A tab dragged along its bar | the tab follows the pointer (held), the tabs it passes slide aside by its width on `Spring::SNAPPY` | `transform`s only: the bar is not laid out again | `features/workspace/src/host/slide.rs` |
| A tab dragged off its bar, over another | its place closes up; the other bar opens a gap where it would go | `transform`s | `slide.rs` `aside` |
| A tab let go | every tab of the bars it touches is moved back by how far its place moved, then sprung to nothing, so nothing jumps; a tab landing in another bar starts there from where it was drawn | `transform`s | `slide.rs` `landed`, `after` |
| A drag that ends nowhere (Escape, outside with nowhere to go) | every tab springs back | | `slide.rs` `cancel` |
| The drop highlight | fades in where a drop would land, follows the pointer from zone to zone on `Spring::TRACK`, fades out when the drag ends (over the split growing in under it) | | `features/workspace/src/host/overlay.rs` `DropHighlight` |
| A split made by a drop | the new group's share grows 0 → its share on `Spring::PANEL`, the others giving way | clipped: laid out at the size the drop lit, held to the drop's edge | `features/workspace/src/host/group.rs` `SplitView`, `features/workspace/src/host/mod.rs` `Host::move_pane` |
| A split's last tab closed from its ✕ | its share folds to 0, then the close lands | clipped at the size it had | `Host::close_from_tab` |
| A divider dragged | 1:1 with the pointer, no motion | | `overlay.rs` `on_move` |
| A phone's tab switcher opening, and closing into a card | the pane's box to its card and back on `Spring::PANEL` (`Switch::zoom`, 0–1): the card is the pane's own view, laid out at the pane's size and `scale`d; the other cards and the bar fade and slide in | `scale` and `left`/`top` of an absolutely placed card (a `translate` is in device pixels on iOS) | `features/workspace/src/host/switcher.rs` |
| A switcher card closed (its ✕, or swiped past half its width or flung) | it fades and shrinks, or flies off at its speed; the cards after it slide into its place, each card's place a pair of motions following its slot on `Spring::SNAPPY` | | `switcher.rs` `CardView`, `close_card`, `on_up` |
| A switcher card lifted (held still) and dragged | it follows the finger (held); the others make room; let go, it springs into its slot | | `switcher.rs` `drop_lifted` |
| The switcher's grid scrolled | held by the finger, past an end it gives half as far; let go, it coasts and springs back inside | | `switcher.rs` `on_move` |
| A stacked strip | a trackpad's pixels move it point for point (macOS sends its coasting as more of them); a wheel's notch (40 points; it was 1) eases there from where it was headed; a spine pressed eases to its pane; a scrollbar flicked coasts | panes slide over the ones before them, each to its spine; a pane's shadow deepens as it slides over | `group.rs` `Stacked`, `features/workspace/src/layout.rs` `slides` |

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
stops it when they rest. The element stays mounted: inserted and removed
with the motions, it collapsed a phone's pane laid out beside it, and a
node made just after it went drew under its animation's opacity (blank)
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

The behaviour follows Obsidian's (sidebars that slide and clip, tabs that
make room, stacked panes that slide over each other to their spines) and
VS Code's and Zed's tab drags. Measuring Obsidian's timings frame by frame
on a Mac is still to do: the study's first run was stopped by a macOS
permission prompt. The springs above are chosen, not measured, and are
the constants to tune against it.
