use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Performance profiler with zero-cost when disabled
pub struct Profiler {
  enabled: bool,
  stats: Arc<ProfileStats>,
}

#[derive(Default)]
pub struct ProfileStats {
  // Resolution
  pub resolution_calls: AtomicUsize,
  pub resolution_cache_hits: AtomicUsize,
  pub resolution_time_ns: AtomicU64,

  // Reference finding
  pub reference_lookups: AtomicUsize,
  pub local_reference_calls: AtomicUsize,
  pub local_reference_time_ns: AtomicU64,

  // Re-export checking
  pub reexport_checks: AtomicUsize,
  pub reexport_time_ns: AtomicU64,

  // Symbol extraction
  pub symbol_extractions: AtomicUsize,
  pub symbol_extraction_time_ns: AtomicU64,
}

impl Profiler {
  /// Create a new profiler
  pub fn new(enabled: bool) -> Self {
    Self {
      enabled,
      stats: Arc::new(ProfileStats::default()),
    }
  }

  /// Check if profiling is enabled (inline for zero-cost check)
  #[inline(always)]
  pub fn is_enabled(&self) -> bool {
    self.enabled
  }

  /// Record a resolution call
  #[inline]
  pub fn record_resolution(&self, cache_hit: bool, duration_ns: u64) {
    if !self.enabled {
      return;
    }
    self.stats.resolution_calls.fetch_add(1, Ordering::Relaxed);
    if cache_hit {
      self
        .stats
        .resolution_cache_hits
        .fetch_add(1, Ordering::Relaxed);
    }
    self
      .stats
      .resolution_time_ns
      .fetch_add(duration_ns, Ordering::Relaxed);
  }

  /// Record a reference lookup
  #[inline]
  pub fn record_reference_lookup(&self) {
    if !self.enabled {
      return;
    }
    self.stats.reference_lookups.fetch_add(1, Ordering::Relaxed);
  }

  /// Record a local reference call
  #[inline]
  pub fn record_local_reference(&self, duration_ns: u64) {
    if !self.enabled {
      return;
    }
    self
      .stats
      .local_reference_calls
      .fetch_add(1, Ordering::Relaxed);
    self
      .stats
      .local_reference_time_ns
      .fetch_add(duration_ns, Ordering::Relaxed);
  }

  /// Record a re-export check
  #[inline]
  #[allow(dead_code)]
  pub fn record_reexport_check(&self, duration_ns: u64) {
    if !self.enabled {
      return;
    }
    self.stats.reexport_checks.fetch_add(1, Ordering::Relaxed);
    self
      .stats
      .reexport_time_ns
      .fetch_add(duration_ns, Ordering::Relaxed);
  }

  /// Record a symbol extraction
  #[inline]
  pub fn record_symbol_extraction(&self, duration_ns: u64) {
    if !self.enabled {
      return;
    }
    self
      .stats
      .symbol_extractions
      .fetch_add(1, Ordering::Relaxed);
    self
      .stats
      .symbol_extraction_time_ns
      .fetch_add(duration_ns, Ordering::Relaxed);
  }

  /// Get the statistics
  pub fn stats(&self) -> &ProfileStats {
    &self.stats
  }

  /// Print profiling report
  pub fn print_report(&self) {
    if !self.enabled {
      return;
    }

    let stats = self.stats();

    eprintln!("\n╔═══════════════════════════════════════════════════════════╗");
    eprintln!("║           PERFORMANCE PROFILING REPORT                    ║");
    eprintln!("╠═══════════════════════════════════════════════════════════╣");

    // Resolution stats
    let resolution_calls = stats.resolution_calls.load(Ordering::Relaxed);
    let cache_hits = stats.resolution_cache_hits.load(Ordering::Relaxed);
    let resolution_time_ms = stats.resolution_time_ns.load(Ordering::Relaxed) / 1_000_000;
    let cache_hit_rate = if resolution_calls > 0 {
      (cache_hits as f64 / resolution_calls as f64) * 100.0
    } else {
      0.0
    };

    eprintln!("║ Module Resolution:                                        ║");
    eprintln!(
      "║   Total calls:        {:>10}                         ║",
      format_number(resolution_calls)
    );
    eprintln!(
      "║   Cache hits:         {:>10} ({:>5.1}%)                ║",
      format_number(cache_hits),
      cache_hit_rate
    );
    eprintln!(
      "║   Time spent:         {:>10} ms                      ║",
      format_number(resolution_time_ms as usize)
    );
    eprintln!(
      "║   Avg per call:       {:>10} μs                      ║",
      if resolution_calls > 0 {
        format_number((resolution_time_ms * 1000 / resolution_calls as u64) as usize)
      } else {
        "0".to_string()
      }
    );
    eprintln!("╠═══════════════════════════════════════════════════════════╣");

    // Reference finding stats
    let reference_lookups = stats.reference_lookups.load(Ordering::Relaxed);
    let local_ref_calls = stats.local_reference_calls.load(Ordering::Relaxed);
    let local_ref_time_ms = stats.local_reference_time_ns.load(Ordering::Relaxed) / 1_000_000;

    eprintln!("║ Reference Finding:                                        ║");
    eprintln!(
      "║   Symbol lookups:     {:>10}                         ║",
      format_number(reference_lookups)
    );
    eprintln!(
      "║   Local ref calls:    {:>10}                         ║",
      format_number(local_ref_calls)
    );
    eprintln!(
      "║   Time spent:         {:>10} ms                      ║",
      format_number(local_ref_time_ms as usize)
    );
    eprintln!(
      "║   Avg per call:       {:>10} μs                      ║",
      if local_ref_calls > 0 {
        format_number((local_ref_time_ms * 1000 / local_ref_calls as u64) as usize)
      } else {
        "0".to_string()
      }
    );
    eprintln!("╠═══════════════════════════════════════════════════════════╣");

    // Re-export stats
    let reexport_checks = stats.reexport_checks.load(Ordering::Relaxed);
    let reexport_time_ms = stats.reexport_time_ns.load(Ordering::Relaxed) / 1_000_000;

    eprintln!("║ Re-export Checking:                                       ║");
    eprintln!(
      "║   Total checks:       {:>10}                         ║",
      format_number(reexport_checks)
    );
    eprintln!(
      "║   Time spent:         {:>10} ms                      ║",
      format_number(reexport_time_ms as usize)
    );
    eprintln!(
      "║   Avg per check:      {:>10} μs                      ║",
      if reexport_checks > 0 {
        format_number((reexport_time_ms * 1000 / reexport_checks as u64) as usize)
      } else {
        "0".to_string()
      }
    );
    eprintln!("╠═══════════════════════════════════════════════════════════╣");

    // Symbol extraction stats
    let symbol_extractions = stats.symbol_extractions.load(Ordering::Relaxed);
    let symbol_time_ms = stats.symbol_extraction_time_ns.load(Ordering::Relaxed) / 1_000_000;

    eprintln!("║ Symbol Extraction:                                        ║");
    eprintln!(
      "║   Total extractions:  {:>10}                         ║",
      format_number(symbol_extractions)
    );
    eprintln!(
      "║   Time spent:         {:>10} ms                      ║",
      format_number(symbol_time_ms as usize)
    );
    eprintln!(
      "║   Avg per extract:    {:>10} μs                      ║",
      if symbol_extractions > 0 {
        format_number((symbol_time_ms * 1000 / symbol_extractions as u64) as usize)
      } else {
        "0".to_string()
      }
    );
    eprintln!("╚═══════════════════════════════════════════════════════════╝");

    // Total time breakdown
    let total_measured_ms =
      resolution_time_ms + local_ref_time_ms + reexport_time_ms + symbol_time_ms;

    if total_measured_ms > 0 {
      eprintln!("\n═══════════════════ TIME BREAKDOWN ═══════════════════");
      eprintln!(
        "Resolution:     {:>6} ms ({:>5.1}%)",
        resolution_time_ms,
        (resolution_time_ms as f64 / total_measured_ms as f64) * 100.0
      );
      eprintln!(
        "Local refs:     {:>6} ms ({:>5.1}%)",
        local_ref_time_ms,
        (local_ref_time_ms as f64 / total_measured_ms as f64) * 100.0
      );
      eprintln!(
        "Re-exports:     {:>6} ms ({:>5.1}%)",
        reexport_time_ms,
        (reexport_time_ms as f64 / total_measured_ms as f64) * 100.0
      );
      eprintln!(
        "Symbol extract: {:>6} ms ({:>5.1}%)",
        symbol_time_ms,
        (symbol_time_ms as f64 / total_measured_ms as f64) * 100.0
      );
      eprintln!("═══════════════════════════════════════════════════════\n");
    }

    // Insights
    eprintln!("💡 INSIGHTS:");
    if cache_hit_rate < 50.0 && resolution_calls > 100 {
      eprintln!(
        "   ⚠️  Low cache hit rate ({:.1}%) - consider pre-resolving imports",
        cache_hit_rate
      );
    }
    if cache_hit_rate > 90.0 {
      eprintln!("   ✅ Excellent cache hit rate ({:.1}%)", cache_hit_rate);
    }

    if resolution_time_ms > local_ref_time_ms {
      eprintln!("   ⚠️  Resolution is the bottleneck - optimize import resolution");
    } else if local_ref_time_ms > resolution_time_ms {
      eprintln!("   ⚠️  Local reference finding is the bottleneck");
    }

    eprintln!();
  }
}

fn format_number(n: usize) -> String {
  let s = n.to_string();
  let mut result = String::new();
  for (i, c) in s.chars().rev().enumerate() {
    if i > 0 && i % 3 == 0 {
      result.push(',');
    }
    result.push(c);
  }
  result.chars().rev().collect()
}

/// Timer guard that records duration when dropped
#[allow(dead_code)]
pub struct TimerGuard<'a, F>
where
  F: FnOnce(u64),
{
  start: Instant,
  callback: Option<F>,
  _phantom: std::marker::PhantomData<&'a ()>,
}

impl<'a, F> TimerGuard<'a, F>
where
  F: FnOnce(u64),
{
  #[allow(dead_code)] // Used by profile_scope! macro
  pub fn new(callback: F) -> Self {
    Self {
      start: Instant::now(),
      callback: Some(callback),
      _phantom: std::marker::PhantomData,
    }
  }
}

impl<'a, F> Drop for TimerGuard<'a, F>
where
  F: FnOnce(u64),
{
  fn drop(&mut self) {
    let duration_ns = self.start.elapsed().as_nanos() as u64;
    if let Some(callback) = self.callback.take() {
      callback(duration_ns);
    }
  }
}

/// Macro to time a block of code (zero-cost when profiler is disabled)
#[macro_export]
macro_rules! profile_scope {
  ($profiler:expr, $method:ident) => {
    let _timer = if $profiler.is_enabled() {
      Some($crate::profiler::TimerGuard::new(|duration_ns| {
        $profiler.$method(duration_ns);
      }))
    } else {
      None
    };
  };
}
