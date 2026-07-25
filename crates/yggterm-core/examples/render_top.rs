//! `render_top` — the read side of the render probe, as a standalone example.
//!
//! Prototype for the `server render-top` command: samples a process tree twice and
//! prints per-role CPU and memory for the interval between the samples. Every number
//! is a delta, so unlike `ps %CPU` (a lifetime average) this shows what the tree is
//! burning *now*.
//!
//! ```sh
//! cargo run -p yggterm-core --example render_top -- <root-pid> [interval-ms]
//! ```

use yggterm_core::render_probe::{RenderProbe, observe_process_tree, roll_up_roles, user_hz};

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root_pid) = args.next().and_then(|value| value.parse::<i32>().ok()) else {
        eprintln!("usage: render_top <root-pid> [interval-ms]");
        std::process::exit(2);
    };
    let interval_ms: u64 = args
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(2000);

    let mut probe = RenderProbe::new();
    let start = std::time::Instant::now();
    let first = observe_process_tree(root_pid);
    if first.is_empty() {
        eprintln!("render_top: no such process tree: {root_pid}");
        std::process::exit(1);
    }
    probe.observe(&first, start.elapsed().as_millis() as u64);

    std::thread::sleep(std::time::Duration::from_millis(interval_ms));

    let second = observe_process_tree(root_pid);
    let samples = probe.observe(&second, start.elapsed().as_millis() as u64);

    println!(
        "render_top: root={root_pid} processes={} interval={interval_ms}ms user_hz={}",
        second.len(),
        user_hz()
    );
    println!(
        "{:<14} {:>6} {:>10} {:>8} {:>12}",
        "role", "procs", "cpu_ms", "cores", "mem_mb"
    );
    let rolled = roll_up_roles(&samples);
    let mut total_cores = 0.0;
    let mut total_mem_mb = 0.0;
    for rollup in &rolled {
        let cores = rollup.core_fraction();
        let mem_mb = rollup.mem_kb as f64 / 1024.0;
        total_cores += cores;
        total_mem_mb += mem_mb;
        println!(
            "{:<14} {:>6} {:>10.1} {cores:>8.3} {mem_mb:>12.1}",
            rollup.role.as_str(),
            rollup.procs,
            rollup.cpu_ms
        );
    }
    println!(
        "{:<14} {:>6} {:>10} {total_cores:>8.3} {total_mem_mb:>12.1}",
        "TOTAL",
        second.len(),
        ""
    );

    println!("\ntop processes by cpu_ms:");
    let mut by_cpu = samples;
    by_cpu.sort_by(|a, b| b.cpu_ms.partial_cmp(&a.cpu_ms).unwrap_or(std::cmp::Ordering::Equal));
    for sample in by_cpu.iter().take(10) {
        let mem_mb = sample.pss_kb.or(sample.rss_kb).unwrap_or(0) as f64 / 1024.0;
        println!(
            "  pid={:<8} {:<16} {:<12} cpu_ms={:>9.1} cores={:>6.3} mem_mb={:>8.1}",
            sample.pid,
            sample.comm,
            sample.role.as_str(),
            sample.cpu_ms,
            sample.core_fraction(),
            mem_mb
        );
    }
}
