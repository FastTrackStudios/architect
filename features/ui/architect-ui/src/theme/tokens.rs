//! CSS variable constants for use in inline styles.
//!
//! These map 1:1 to the custom properties defined in `fts-theme.css`.
//! Use these instead of hardcoded strings so typos become compile errors.

// --- Core palette ---
pub const BACKGROUND: &str = "var(--background)";
pub const FOREGROUND: &str = "var(--foreground)";
pub const CARD: &str = "var(--card)";
pub const CARD_FOREGROUND: &str = "var(--card-foreground)";
pub const POPOVER: &str = "var(--popover)";
pub const POPOVER_FOREGROUND: &str = "var(--popover-foreground)";
pub const PRIMARY: &str = "var(--primary)";
pub const PRIMARY_FOREGROUND: &str = "var(--primary-foreground)";
pub const SECONDARY: &str = "var(--secondary)";
pub const SECONDARY_FOREGROUND: &str = "var(--secondary-foreground)";
pub const MUTED: &str = "var(--muted)";
pub const MUTED_FOREGROUND: &str = "var(--muted-foreground)";
pub const ACCENT: &str = "var(--accent)";
pub const ACCENT_FOREGROUND: &str = "var(--accent-foreground)";
pub const DESTRUCTIVE: &str = "var(--destructive)";
pub const DESTRUCTIVE_FOREGROUND: &str = "var(--destructive-foreground)";
pub const BORDER: &str = "var(--border)";
pub const INPUT: &str = "var(--input)";
pub const RING: &str = "var(--ring)";

// --- Radius ---
pub const RADIUS: &str = "var(--radius)";

// --- Sidebar ---
pub const SIDEBAR: &str = "var(--sidebar)";
pub const SIDEBAR_FOREGROUND: &str = "var(--sidebar-foreground)";
pub const SIDEBAR_PRIMARY: &str = "var(--sidebar-primary)";
pub const SIDEBAR_PRIMARY_FOREGROUND: &str = "var(--sidebar-primary-foreground)";
pub const SIDEBAR_ACCENT: &str = "var(--sidebar-accent)";
pub const SIDEBAR_ACCENT_FOREGROUND: &str = "var(--sidebar-accent-foreground)";
pub const SIDEBAR_BORDER: &str = "var(--sidebar-border)";
pub const SIDEBAR_RING: &str = "var(--sidebar-ring)";

// --- Signal semantic colors ---
pub const SIGNAL_SAFE: &str = "var(--signal-safe)";
pub const SIGNAL_WARN: &str = "var(--signal-warn)";
pub const SIGNAL_DANGER: &str = "var(--signal-danger)";
pub const SIGNAL_OVERRIDE: &str = "var(--signal-override)";
pub const SIGNAL_AUTOMATION: &str = "var(--signal-automation)";
pub const SIGNAL_MOD: &str = "var(--signal-mod)";
pub const SIGNAL_SLOT_A: &str = "var(--signal-slot-a)";
pub const SIGNAL_SLOT_B: &str = "var(--signal-slot-b)";
pub const SIGNAL_QUICK: &str = "var(--signal-quick)";
pub const SIGNAL_FULL: &str = "var(--signal-full)";
