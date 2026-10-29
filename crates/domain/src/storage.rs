//! Storage facts shared by the archive and usage accounting.
/// Raw capture audio stays untouched this long, then becomes a verified lossless copy.
pub const RAW_AUDIO_RETENTION_DAYS: u64 = 7;
