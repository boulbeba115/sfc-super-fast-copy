//! Number formatting shared by the CLI and GUI.

use std::time::Duration;

pub fn bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

pub fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn duration(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        format!("{s:.1} s")
    } else if s < 3600.0 {
        format!("{}m {:02}s", s as u64 / 60, s as u64 % 60)
    } else {
        format!("{}h {:02}m", s as u64 / 3600, (s as u64 % 3600) / 60)
    }
}

pub fn rate(amount: u64, d: Duration) -> f64 {
    let s = d.as_secs_f64();
    if s > 0.0 {
        amount as f64 / s
    } else {
        0.0
    }
}
