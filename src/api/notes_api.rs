use crate::api::client::ApiClient;
use crate::api::types::{
    ApiErrorResponse, NotesBatchRequest, NotesBatchResponse, NotesListRequest, NotesListResponse,
    NotesPushRequest, NotesPushResponse,
};
use crate::error::GitAiError;

fn parse_api_error_message(body: &str, fallback: &str) -> String {
    serde_json::from_str::<ApiErrorResponse>(body)
        .map(|e| e.error)
        .unwrap_or_else(|_| fallback.to_string())
}

impl ApiClient {
    pub fn notes_list(&self, request: &NotesListRequest) -> Result<NotesListResponse, GitAiError> {
        let response = self.context().post_json("/api/v1/notes/list", request)?;
        let status_code = response.status_code;
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        if status_code != 200 {
            let message = parse_api_error_message(body, "Notes list request failed");
            return Err(GitAiError::Generic(format!(
                "Notes list failed with status {}: {}",
                status_code, message
            )));
        }

        let parsed: NotesListResponse =
            serde_json::from_str(body).map_err(GitAiError::JsonError)?;
        if !parsed.ok {
            return Err(GitAiError::Generic(
                "Notes list returned ok=false".to_string(),
            ));
        }
        Ok(parsed)
    }

    pub fn notes_batch_get(
        &self,
        request: &NotesBatchRequest,
    ) -> Result<NotesBatchResponse, GitAiError> {
        let response = self.context().post_json("/api/v1/notes/batch", request)?;
        let status_code = response.status_code;
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        if status_code != 200 {
            let message = parse_api_error_message(body, "Notes batch request failed");
            return Err(GitAiError::Generic(format!(
                "Notes batch failed with status {}: {}",
                status_code, message
            )));
        }

        let parsed: NotesBatchResponse =
            serde_json::from_str(body).map_err(GitAiError::JsonError)?;
        if !parsed.ok {
            return Err(GitAiError::Generic(
                "Notes batch returned ok=false".to_string(),
            ));
        }
        Ok(parsed)
    }

    pub fn notes_push(&self, request: &NotesPushRequest) -> Result<NotesPushResponse, GitAiError> {
        let response = self.context().post_json("/api/v1/notes/push", request)?;
        let status_code = response.status_code;
        let body = response
            .as_str()
            .map_err(|e| GitAiError::Generic(format!("Failed to read response body: {}", e)))?;

        if status_code != 200 {
            let message = parse_api_error_message(body, "Notes push request failed");
            return Err(GitAiError::Generic(format!(
                "Notes push failed with status {}: {}",
                status_code, message
            )));
        }

        let parsed: NotesPushResponse =
            serde_json::from_str(body).map_err(GitAiError::JsonError)?;
        if !parsed.ok {
            return Err(GitAiError::Generic(
                "Notes push returned ok=false".to_string(),
            ));
        }
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::parse_api_error_message;

    #[test]
    fn parse_api_error_message_prefers_error_field() {
        let body = r#"{"error":"boom","details":{"x":1}}"#;
        let message = parse_api_error_message(body, "fallback");
        assert_eq!(message, "boom");
    }

    #[test]
    fn parse_api_error_message_falls_back_on_invalid_json() {
        let message = parse_api_error_message("not-json", "fallback");
        assert_eq!(message, "fallback");
    }
}
