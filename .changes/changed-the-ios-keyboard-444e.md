---
"architect-shell" = "minor"
---

Changed the iOS keyboard to come up only when asked: a tap that ends on a field (`TapStroke`, `tap_raises`), the action bar's new keyboard button (`Keyboard::foot`, `Foot`) or `show_keyboard`; a press that becomes a scroll and an `autofocus` no longer raise it, and `set_typing` only reports the focus. `hide_keyboard` puts it away whatever holds the focus (the editing bar's Hide button). Added `set_input_debug` / `ARCHITECT_INPUT_DEBUG=1`: the touches, the keyboard's reported top and the foot's laid-out edge drawn over the app (`foot_bottom`)
