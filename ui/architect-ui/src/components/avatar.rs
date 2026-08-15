//! Avatar — shadcn v4 maia style.

use dioxus::prelude::*;
use dioxus_primitives::avatar::{
    Avatar as PrimitiveAvatar, AvatarFallback as PrimitiveAvatarFallback,
    AvatarImage as PrimitiveAvatarImage, AvatarState,
};
use architect_story_runtime::story;

/// Size variant for the avatar.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum AvatarSize {
    Small,
    #[default]
    Medium,
    Large,
}

impl AvatarSize {
    fn classes(self) -> &'static str {
        match self {
            Self::Small => "size-6",
            Self::Medium => "size-8",
            Self::Large => "size-10",
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AvatarProps {
    #[props(default)]
    pub size: AvatarSize,
    #[props(default)]
    pub on_load: Option<EventHandler<()>>,
    #[props(default)]
    pub on_error: Option<EventHandler<()>>,
    #[props(default)]
    pub on_state_change: Option<EventHandler<AvatarState>>,
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// shadcn v4 maia: cn-avatar
#[component]
pub fn Avatar(props: AvatarProps) -> Element {
    let base = "relative flex shrink-0 overflow-hidden rounded-full";
    let size_class = props.size.classes();

    rsx! {
        PrimitiveAvatar {
            on_load: props.on_load,
            on_error: props.on_error,
            on_state_change: props.on_state_change,
            class: crate::cn::merge_slice(&[base, size_class, props.class.as_str()]),
            {props.children}
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AvatarImageProps {
    pub src: String,
    #[props(default)]
    pub alt: Option<String>,
    #[props(default)]
    pub class: String,
}

/// Image inside an Avatar. Positioned absolutely so it covers the fallback when loaded.
#[component]
pub fn AvatarImage(props: AvatarImageProps) -> Element {
    rsx! {
        PrimitiveAvatarImage {
            class: crate::cn::merge_slice(&["absolute inset-0 aspect-square h-full w-full rounded-full object-cover", props.class.as_str()]),
            src: props.src,
            alt: props.alt,
        }
    }
}

#[derive(Props, Clone, PartialEq)]
pub struct AvatarFallbackProps {
    #[props(default)]
    pub class: String,
    pub children: Element,
}

/// Fallback content (e.g. initials) shown behind the avatar image.
#[component]
pub fn AvatarFallback(props: AvatarFallbackProps) -> Element {
    let base = "flex h-full w-full items-center justify-center rounded-full bg-muted text-muted-foreground text-xs font-medium";

    rsx! {
        PrimitiveAvatarFallback {
            class: crate::cn::merge_slice(&[base, props.class.as_str()]),
            {props.children}
        }
    }
}

/// Avatar fallback paths: image-loaded / image-failed (broken URL) / initials-only.
#[story(category = "Avatar", name = "fallbacks")]
pub fn avatar_fallbacks() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-6",
            div { class: "flex flex-col items-center gap-2",
                Avatar {
                    AvatarImage { src: "https://i.pravatar.cc/64?u=loaded".to_string(), alt: "User".to_string() }
                    AvatarFallback { "OK" }
                }
                span { class: "text-xs text-muted-foreground", "image loaded" }
            }
            div { class: "flex flex-col items-center gap-2",
                Avatar {
                    AvatarImage {
                        src: "https://invalid.example.invalid/missing.png".to_string(),
                        alt: "User".to_string(),
                    }
                    AvatarFallback { "FB" }
                }
                span { class: "text-xs text-muted-foreground", "image failed" }
            }
            div { class: "flex flex-col items-center gap-2",
                Avatar {
                    AvatarFallback { "JD" }
                }
                span { class: "text-xs text-muted-foreground", "initials only" }
            }
        }
    }
}

/// Avatar sizes side-by-side, with image, fallback initials, and error fallback.
#[story(category = "Avatar", name = "sizes")]
pub fn avatar_sizes() -> Element {
    rsx! {
        div { class: "p-6 bg-background text-foreground flex items-center gap-4",
            Avatar { size: AvatarSize::Small,
                AvatarFallback { "S" }
            }
            Avatar { size: AvatarSize::Medium,
                AvatarFallback { "M" }
            }
            Avatar { size: AvatarSize::Large,
                AvatarFallback { "LG" }
            }
            Avatar {
                AvatarImage { src: "https://i.pravatar.cc/64?u=fts".to_string(), alt: "User".to_string() }
                AvatarFallback { "FT" }
            }
        }
    }
}
