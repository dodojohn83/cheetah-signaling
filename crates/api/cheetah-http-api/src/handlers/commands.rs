//! Typed device command HTTP handlers.
//!
//! These endpoints wrap `OperationService::submit_operation` and accept typed
//! request bodies for PTZ, preset, query and device-control commands.  Each
//! command becomes an `Operation` with a `CommandPayload` that the protocol
//! module dispatches to the device.

#![allow(missing_docs)]

use crate::{ApiRequestContext, ApiState, HttpError, IdempotencyKey, JsonBody};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderValue, StatusCode, header},
    response::IntoResponse,
};
use cheetah_domain::{CommandPayload, DeviceControlKind, PtzDirection, QueryKind};
use cheetah_signal_application::dto::SubmitOperationRequest;
use cheetah_signal_types::{
    ChannelId, Deadline, DeviceId, DurationMs, OwnerEpoch, ResourceId, ResourceKind, ResourceRef,
    UtcTimestamp,
};
use serde::Deserialize;
use std::sync::Arc;

/// Optional deadline and owner epoch overrides for command endpoints.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct CommandOptions {
    /// RFC 3339 deadline.  Defaults to 30 seconds from now when omitted.
    pub deadline: Option<String>,
    /// Expected owner epoch for fencing.  Defaults to 1 when omitted.
    pub owner_epoch: Option<u64>,
}

/// PTZ command request body.
#[derive(Clone, Debug, Deserialize)]
pub struct PtzCommandRequest {
    /// Channel to control.
    pub channel_id: ChannelId,
    /// Direction of movement.
    pub direction: PtzDirection,
    /// Speed factor.
    pub speed: f64,
}

/// Preset command request body.
#[derive(Clone, Debug, Deserialize)]
pub struct PresetCommandRequest {
    /// Channel that owns the preset.
    pub channel_id: ChannelId,
    /// Preset action.
    pub action: cheetah_domain::PresetAction,
    /// Preset identifier.
    pub preset_id: u32,
}

/// Query command request body.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct QueryCommandRequest {
    /// Kind of query.
    pub kind: QueryKind,
    /// Optional target channel.
    pub channel_id: Option<ChannelId>,
    /// Optional RFC 3339 start time.
    pub start_time: Option<String>,
    /// Optional RFC 3339 end time.
    pub end_time: Option<String>,
    /// Optional configuration type for `ConfigDownload`.
    pub config_type: Option<String>,
    /// Optional playback scale for `RecordInfo`.
    pub scale: Option<f64>,
}

/// Device control request body.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct DeviceControlCommandRequest {
    /// Kind of control action.
    pub kind: DeviceControlKind,
    /// Optional target channel.
    pub channel_id: Option<ChannelId>,
    /// Boolean parameter for toggle actions.
    pub enabled: Option<bool>,
    /// Opaque string parameter.
    pub param: Option<String>,
}

fn build_target(tenant_id: cheetah_signal_types::TenantId, device_id: DeviceId) -> ResourceRef {
    ResourceRef {
        tenant_id,
        kind: ResourceKind::Device,
        id: ResourceId::Device(device_id),
    }
}

fn build_deadline(
    state: &ApiState,
    options: &CommandOptions,
) -> Result<Option<Deadline>, HttpError> {
    if let Some(deadline_str) = &options.deadline {
        let ts = UtcTimestamp::parse_rfc3339(deadline_str).map_err(HttpError::from)?;
        return Ok(Some(Deadline::from_timestamp(ts)));
    }
    Ok(Deadline::from_now(
        state.clock.now_wall(),
        DurationMs::from_seconds(30),
    ))
}

fn owner_epoch(options: &CommandOptions) -> OwnerEpoch {
    OwnerEpoch(options.owner_epoch.unwrap_or(1))
}

macro_rules! submit_command {
    ($state:expr, $ctx:expr, $uow:expr, $device_id:expr, $payload:expr, $idempotency:expr, $options:expr) => {{
        let tenant_id = $ctx.0.tenant_id;
        let request = SubmitOperationRequest {
            device_id: $device_id,
            target: build_target(tenant_id, $device_id),
            payload: $payload,
            idempotency_key: $idempotency.0,
            deadline: build_deadline(&$state, &$options)?,
            expected_owner_epoch: owner_epoch(&$options),
        };
        $state
            .operation_service
            .submit_operation(&$ctx.0, $uow, request)
    }};
}

pub async fn ptz(
    Path(device_id): Path<String>,
    Query(options): Query<CommandOptions>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(request): JsonBody<PtzCommandRequest>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = submit_command!(
        state,
        ctx,
        &mut *uow,
        device_id,
        CommandPayload::Ptz {
            channel_id: request.channel_id,
            direction: request.direction,
            speed: request.speed,
        },
        idempotency,
        options
    )
    .await
    .map_err(HttpError::from)?;
    let operation_id = operation.operation_id.to_string();
    let mut response = (StatusCode::ACCEPTED, Json(operation)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/operations/{operation_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

pub async fn preset(
    Path(device_id): Path<String>,
    Query(options): Query<CommandOptions>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(request): JsonBody<PresetCommandRequest>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = submit_command!(
        state,
        ctx,
        &mut *uow,
        device_id,
        CommandPayload::Preset {
            preset: cheetah_domain::PresetCommand {
                channel_id: request.channel_id,
                action: request.action,
                preset_id: request.preset_id,
            },
        },
        idempotency,
        options
    )
    .await
    .map_err(HttpError::from)?;
    let operation_id = operation.operation_id.to_string();
    let mut response = (StatusCode::ACCEPTED, Json(operation)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/operations/{operation_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

pub async fn query(
    Path(device_id): Path<String>,
    Query(options): Query<CommandOptions>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(request): JsonBody<QueryCommandRequest>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let start_time = request
        .start_time
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let end_time = request
        .end_time
        .as_deref()
        .map(UtcTimestamp::parse_rfc3339)
        .transpose()
        .map_err(HttpError::from)?;
    let operation = submit_command!(
        state,
        ctx,
        &mut *uow,
        device_id,
        CommandPayload::Query {
            query: cheetah_domain::QueryCommand {
                kind: request.kind,
                channel_id: request.channel_id,
                start_time,
                end_time,
                config_type: request.config_type,
                scale: request.scale,
            },
        },
        idempotency,
        options
    )
    .await
    .map_err(HttpError::from)?;
    let operation_id = operation.operation_id.to_string();
    let mut response = (StatusCode::ACCEPTED, Json(operation)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/operations/{operation_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}

pub async fn device_control(
    Path(device_id): Path<String>,
    Query(options): Query<CommandOptions>,
    State(state): State<Arc<ApiState>>,
    ctx: ApiRequestContext,
    idempotency: IdempotencyKey,
    JsonBody(request): JsonBody<DeviceControlCommandRequest>,
) -> Result<axum::response::Response, HttpError> {
    ctx.require_scope("operator")?;
    let device_id = device_id.parse::<DeviceId>().map_err(HttpError::from)?;
    let mut uow = state.storage.begin().await.map_err(HttpError::from)?;
    let operation = submit_command!(
        state,
        ctx,
        &mut *uow,
        device_id,
        CommandPayload::DeviceControl {
            control: cheetah_domain::DeviceControlCommand {
                kind: request.kind,
                channel_id: request.channel_id,
                enabled: request.enabled,
                param: request.param,
            },
        },
        idempotency,
        options
    )
    .await
    .map_err(HttpError::from)?;
    let operation_id = operation.operation_id.to_string();
    let mut response = (StatusCode::ACCEPTED, Json(operation)).into_response();
    if let Ok(value) = HeaderValue::from_str(&format!("/api/v1/operations/{operation_id}")) {
        response.headers_mut().insert(header::LOCATION, value);
    }
    Ok(response)
}
