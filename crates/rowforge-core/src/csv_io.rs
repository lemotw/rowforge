// P11: All dead code removed. CsvInput, CsvReadError, OutputWriter, CsvWriteError
// and the write_success/write_failed helpers were deleted (superseded by
// input_stream::CsvInputStream and exec export, respectively).
//
// FieldMap now lives in reader.rs (its primary consumer); this module re-exports
// it here for backward compatibility with external callers using
// `rowforge_core::csv_io::FieldMap`.
pub use crate::reader::FieldMap;
