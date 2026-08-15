//! Direction provider — ltr/rtl context and root `dir` attribute.

use dioxus::prelude::*;

/// Text direction for bidirectional layouts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Direction {
    #[default]
    Ltr,
    Rtl,
}

impl Direction {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ltr => "ltr",
            Self::Rtl => "rtl",
        }
    }

    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DirectionContext {
    pub direction: Direction,
}

pub fn use_direction() -> Direction {
    use_context::<DirectionContext>().direction
}

#[derive(Props, Clone, PartialEq)]
pub struct DirectionProviderProps {
    #[props(default)]
    pub direction: Direction,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

#[component]
pub fn DirectionProvider(props: DirectionProviderProps) -> Element {
    use_context_provider(|| DirectionContext {
        direction: props.direction,
    });

    rsx! {
        div {
            dir: props.direction.as_str(),
            class: crate::cn::merge_slice(&["contents", props.class.as_str()]),
            {props.children}
        }
    }
}
