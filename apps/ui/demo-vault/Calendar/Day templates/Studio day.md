---
type: day-template
name: Studio day
color: "#8b5cf6"
description: One long session, meals prepped ahead.
blocks:
  - {id: reset, start: "07:00", end: "07:30", label: Morning reset, category: reset}
  - {id: breakfast, start: "07:30", end: "08:00", label: Breakfast prep, category: meal}
  - {id: session, start: "09:00", end: "13:00", label: Studio session, category: allocatable, fixed: true}
  - {id: lunch, start: "13:00", end: "13:45", label: Lunch prep, category: meal}
  - {id: block-2, start: "14:00", end: "18:00", label: Block 2, category: allocatable}
  - {id: dinner, start: "18:00", end: "19:00", label: Dinner prep, category: meal}
  - {id: wind-down, start: "21:30", end: "22:30", label: Wind-down, category: wind_down}
  - {id: sleep, start: "22:30", end: "07:00", label: Sleep, category: sleep}
---
# Studio day

One long session, meals prepped ahead.
