// v0.0.1 - Authenticate both directions of the private attachment metadata endpoint.
use super::*;
use crate::private_media_protocol::{
    PrivateMediaExchangeOutcome, PrivateMediaQuery, PrivateMediaRequest, VerifiedPrivateMediaReply,
    PRIVATE_MEDIA_MAX_REQUEST_BYTES,
};

pub(super) fn handle(request: &HttpRequest, store: &SqliteServerStore) -> (u16, SyncClientResult) {
    let Some(raw_token) = bearer_token(request) else {
        return (401, error_result("Missing sync token."));
    };
    let authenticated = match authenticate_token_request(store, raw_token, now_millis()) {
        Ok(token) => token,
        Err(error) => return error,
    };
    if request.body.len() > PRIVATE_MEDIA_MAX_REQUEST_BYTES {
        return (
            413,
            error_result("Private attachment request exceeds its byte limit."),
        );
    }
    let body: PrivateMediaRequest = match serde_json::from_str(&request.body) {
        Ok(value) => value,
        Err(_) => return (400, error_result("Invalid private attachment request.")),
    };
    let user = &authenticated.user_id;
    match store.exchange_private_media_references(user, authenticated.token_id, &body, now_millis())
    {
        Ok(reply) => (
            200,
            SyncClientResult {
                ok: true,
                message: "Private attachment references exchanged.".into(),
                user_id: user.clone(),
                current_generation: reply.generation,
                mode: "private_media_exchange".into(),
                private_media_exchange: Some(reply),
                ..Default::default()
            },
        ),
        Err(StoreError::RestoreGenerationConflict {
            actual_generation, ..
        })
        | Err(StoreError::RestoreReceiptRequired { actual_generation }) => {
            media_restore_required_response(store, user, authenticated.token_id, actual_generation)
        }
        Err(StoreError::ServerGenerationRollback {
            server_generation, ..
        }) => server_generation_rollback_response(user, server_generation),
        Err(StoreError::Integrity(message)) => (400, error_result(&message)),
        Err(error) => store_failure("Could not exchange private attachment references", error),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn exchange_client(
    server_url: &str,
    user: &str,
    token: &str,
    server: &str,
    namespace: &str,
    generation: i64,
    receipt: &str,
    queries: Vec<PrivateMediaQuery>,
) -> PrivateMediaExchangeOutcome {
    let failed = |result| PrivateMediaExchangeOutcome {
        result,
        verified: None,
    };
    if let Some(error) = validate_desktop_media_auth(user, token, server, namespace) {
        return failed(error);
    }
    let request = PrivateMediaRequest {
        format_version: 1,
        request_id: format!("private_{}", random_token(24)),
        acknowledged_generation: generation,
        restore_receipt: receipt.to_owned(),
        queries,
    };
    if let Err(error) = request.validate() {
        return failed(desktop_media_client_error(&error));
    }
    let response = post_json(
        server_url,
        "/v1/media/private-references",
        Some(token),
        serde_json::to_value(&request).expect("private reference request is serializable"),
    );
    validate_client_response(response, &request, user, token, server, namespace)
}

pub(super) fn validate_client_response(
    response: SyncClientResult,
    request: &PrivateMediaRequest,
    user: &str,
    token: &str,
    server: &str,
    namespace: &str,
) -> PrivateMediaExchangeOutcome {
    let failed = |result| PrivateMediaExchangeOutcome {
        result,
        verified: None,
    };
    let result =
        match validate_desktop_media_response_identity(response, user, token, server, namespace) {
            Ok(response) => response,
            Err(error) => return failed(error),
        };
    if !result.ok {
        return failed(result);
    }
    let Some(reply) = &result.private_media_exchange else {
        return failed(desktop_media_client_error(
            "Private attachment response payload is missing.",
        ));
    };
    if result.mode != "private_media_exchange"
        || result.restore_required
        || result.baseline_merge_required
        || result.current_generation != request.acknowledged_generation
    {
        return failed(desktop_media_client_error(
            "Private attachment response belongs to another restore state.",
        ));
    }
    if let Err(error) = reply.validate_for(request) {
        return failed(desktop_media_client_error(&error));
    }
    let verified = VerifiedPrivateMediaReply {
        user_id: user.to_owned(),
        token_id: token_identifier(token),
        server_instance_id: server.to_owned(),
        account_namespace: namespace.to_owned(),
        reply: reply.clone(),
    };
    PrivateMediaExchangeOutcome {
        result,
        verified: Some(verified),
    }
}
