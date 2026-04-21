use crate::observability::flush;

pub fn handle_flush_logs(args: &[String]) {
    flush::handle_flush_logs(args);
}
