//! Offline block render engine: 64-sample blocks split at event boundaries,
//! pure voice scheduling, per-track render (the parallelism unit and the free
//! stems export), f64 master sum in fixed track order, WAV out via hound.
