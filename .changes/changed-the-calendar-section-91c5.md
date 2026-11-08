---
"architect-navigator" = "minor"
---

Changed the Calendar section (a top mount with a month grid) to sit pinned at the sidebar's foot, under the tree, where Notebook Navigator keeps its calendar, instead of second among the top rows; added `NavigatorState::foot_rows` and `row_by_key`. Its row and its days still take drops. The calendar's weekend shade (`navigator.calendar-weekend`) is the text at 10%, as Notebook Navigator's (was 4%).
