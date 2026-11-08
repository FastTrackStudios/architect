---
type: day-template-rules
rules:
  - {template: Gig day, tag: gig}
  - {template: Studio day, rrule: "FREQ=WEEKLY;INTERVAL=2;BYDAY=SA", start: 2026-10-31}
  - {template: Workday, weekdays: [mon, tue, wed, thu, fri]}
fallback: Rest day
overrides:
  - {from: 2026-11-12, to: 2026-11-12, span: day, template: Studio day}
---
# Day rules

Which day template each day uses: the first rule that matches, unless an override says otherwise.
