use crate::{error::AppError, storage::sqlite::DuckDbRepository, types::meetings::MeetingCalendar};
use tauri::State;

/// Meetings recognized in ingested documents, with cited suggestions for upcoming agendas.
#[tauri::command]
pub async fn get_meeting_calendar(
    repository: State<'_, DuckDbRepository>,
) -> Result<MeetingCalendar, AppError> {
    crate::research::meetings::calendar(&repository, chrono::Local::now().date_naive()).await
}
