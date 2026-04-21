pub const MOTION_STANDARD: &str = "cubic-bezier(0.2, 0, 0, 1)";
pub const MOTION_STANDARD_DECELERATE: &str = "cubic-bezier(0, 0, 0, 1)";
pub const MOTION_STANDARD_ACCELERATE: &str = "cubic-bezier(0.3, 0, 1, 1)";
pub const MOTION_EMPHASIZED_DECELERATE: &str = "cubic-bezier(0.05, 0.7, 0.1, 1)";
pub const MOTION_EMPHASIZED_ACCELERATE: &str = "cubic-bezier(0.3, 0, 0.8, 0.15)";

pub const MOTION_STANDARD_DURATION_MS: u32 = 190;
pub const MOTION_ENTER_DURATION_MS: u32 = 210;
pub const MOTION_EXIT_DURATION_MS: u32 = 170;

pub fn transition(properties: &[&str], duration_ms: u32, easing: &str) -> String {
    properties
        .iter()
        .map(|property| format!("{property} {duration_ms}ms {easing}"))
        .collect::<Vec<_>>()
        .join(", ")
}

pub fn standard_transition(properties: &[&str]) -> String {
    transition(properties, MOTION_STANDARD_DURATION_MS, MOTION_STANDARD)
}

pub fn standard_decelerate_transition(properties: &[&str]) -> String {
    transition(
        properties,
        MOTION_STANDARD_DURATION_MS,
        MOTION_STANDARD_DECELERATE,
    )
}

pub fn standard_accelerate_transition(properties: &[&str]) -> String {
    transition(
        properties,
        MOTION_STANDARD_DURATION_MS,
        MOTION_STANDARD_ACCELERATE,
    )
}

pub fn emphasized_enter_transition(properties: &[&str]) -> String {
    transition(
        properties,
        MOTION_ENTER_DURATION_MS,
        MOTION_EMPHASIZED_DECELERATE,
    )
}

pub fn emphasized_exit_transition(properties: &[&str]) -> String {
    transition(
        properties,
        MOTION_EXIT_DURATION_MS,
        MOTION_EMPHASIZED_ACCELERATE,
    )
}
