use std::{collections::HashMap, fs};

use base64::prelude::*;
use protobuf::{
    EnumOrUnknown, Message as _, MessageField, SpecialFields,
    well_known_types::timestamp::Timestamp,
};
use protogen::{
    system_event::{
        SystemEvent,
        system_event::{Event as SystemEventVariant, MouseButton, MouseDown},
    },
    trace_bundle::{
        TraceBundle, TraceReplayRequest,
        trace_bundle::{
            Alert, Attribute, Environment, Event as TraceEvent, Header, HttpRequest, QueueMessage,
            Service, Severity, Span, SpanStatus, Transport as TraceTransport, UserContext,
            attribute::Value as AttributeValue,
        },
        trace_replay_request::Source as ReplaySource,
    },
};

fn main() {
    write_sample("system-event", system_event_sample());
    write_sample("trace-bundle-http", trace_bundle_http_sample());
    write_sample("trace-bundle-queue", trace_bundle_queue_sample());
    write_sample("trace-replay-request", trace_replay_request_sample());
}

fn system_event_sample() -> Vec<u8> {
    SystemEvent {
        timestamp: MessageField::some(Timestamp {
            seconds: 1_234_567,
            nanos: 123,
            special_fields: SpecialFields::default(),
        }),
        reason: Some("user clicked".to_owned()),
        event: Some(SystemEventVariant::Click(MouseDown {
            button: EnumOrUnknown::new(MouseButton::Left),
            x: 42,
            y: 100,
            ..Default::default()
        })),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap()
}

fn trace_bundle_http_sample() -> Vec<u8> {
    TraceBundle {
        export_id: "exp-http-20260326".to_owned(),
        captured_at: MessageField::some(timestamp(1_711_465_600, 987_654_321)),
        environment: EnumOrUnknown::new(Environment::Production),
        services: vec![
            Service {
                name: "frontend".to_owned(),
                version: "2026.03.26".to_owned(),
                spans: vec![
                    Span {
                        span_id: "span-root".to_owned(),
                        started_at: MessageField::some(timestamp(1_711_465_500, 111_000_000)),
                        duration_ms: 184,
                        status: EnumOrUnknown::new(SpanStatus::Error),
                        attributes: vec![
                            string_attribute("component", "ui"),
                            int_attribute("http.status_code", 500),
                            bool_attribute("cache.hit", false),
                        ],
                        events: vec![
                            TraceEvent {
                                name: "db.retry".to_owned(),
                                at: MessageField::some(timestamp(1_711_465_500, 333_000_000)),
                                fields: vec![
                                    string_attribute("system", "postgres"),
                                    int_attribute("attempt", 2),
                                ],
                                special_fields: SpecialFields::default(),
                            },
                            TraceEvent {
                                name: "exception".to_owned(),
                                at: MessageField::some(timestamp(1_711_465_500, 444_000_000)),
                                fields: vec![
                                    string_attribute("type", "TimeoutError"),
                                    bytes_attribute("fingerprint", b"db:users:timeout"),
                                ],
                                special_fields: SpecialFields::default(),
                            },
                        ],
                        ..Default::default()
                    },
                    Span {
                        span_id: "span-child-http".to_owned(),
                        parent_span_id: Some("span-root".to_owned()),
                        started_at: MessageField::some(timestamp(1_711_465_500, 222_000_000)),
                        duration_ms: 97,
                        status: EnumOrUnknown::new(SpanStatus::Ok),
                        attributes: vec![
                            string_attribute("http.method", "POST"),
                            string_attribute("route", "/api/orders"),
                        ],
                        events: vec![],
                        ..Default::default()
                    },
                ],
                annotations: HashMap::from([
                    ("deployment.region".to_owned(), "eu-central".to_owned()),
                    ("team".to_owned(), "checkout".to_owned()),
                ]),
                special_fields: SpecialFields::default(),
            },
            Service {
                name: "billing-worker".to_owned(),
                version: "2026.03.25.4".to_owned(),
                spans: vec![Span {
                    span_id: "span-worker".to_owned(),
                    parent_span_id: Some("span-root".to_owned()),
                    started_at: MessageField::some(timestamp(1_711_465_500, 555_000_000)),
                    duration_ms: 412,
                    status: EnumOrUnknown::new(SpanStatus::Timeout),
                    attributes: vec![string_attribute("queue", "billing-jobs")],
                    events: vec![],
                    ..Default::default()
                }],
                annotations: HashMap::from([("runtime".to_owned(), "rust".to_owned())]),
                special_fields: SpecialFields::default(),
            },
        ],
        labels: HashMap::from([
            ("cluster".to_owned(), "prod-eu-1".to_owned()),
            ("incident".to_owned(), "INC-2048".to_owned()),
        ]),
        user: MessageField::some(UserContext {
            user_id: "user-42".to_owned(),
            roles: vec!["admin".to_owned(), "support".to_owned()],
            traits: HashMap::from([
                ("locale".to_owned(), "en-GB".to_owned()),
                ("plan".to_owned(), "enterprise".to_owned()),
            ]),
            special_fields: SpecialFields::default(),
        }),
        alerts: vec![
            Alert {
                severity: EnumOrUnknown::new(Severity::Critical),
                code: "HTTP_500_BURST".to_owned(),
                related_span_ids: vec!["span-root".to_owned(), "span-child-http".to_owned()],
                summary: Some("checkout API returned repeated 500s".to_owned()),
                special_fields: SpecialFields::default(),
            },
            Alert {
                severity: EnumOrUnknown::new(Severity::Warning),
                code: "WORKER_TIMEOUT".to_owned(),
                related_span_ids: vec!["span-worker".to_owned()],
                summary: Some("billing worker exceeded SLA".to_owned()),
                special_fields: SpecialFields::default(),
            },
        ],
        transport: Some(TraceTransport::Http(HttpRequest {
            method: "POST".to_owned(),
            path: "/ingest/traces".to_owned(),
            headers: vec![
                header("content-type", "application/x-protobuf"),
                header("x-request-id", "req-7bdb7"),
                header("x-forwarded-for", "203.0.113.9"),
            ],
            body: b"{\"batch\":2,\"compressed\":false}".to_vec(),
            special_fields: SpecialFields::default(),
        })),
        raw_envelope: b"trace-http-envelope".to_vec(),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap()
}

fn trace_bundle_queue_sample() -> Vec<u8> {
    TraceBundle {
        export_id: "exp-queue-20260326".to_owned(),
        captured_at: MessageField::some(timestamp(1_711_469_200, 12_000_000)),
        environment: EnumOrUnknown::new(Environment::Staging),
        services: vec![Service {
            name: "inventory-sync".to_owned(),
            version: "1.17.0-rc1".to_owned(),
            spans: vec![
                Span {
                    span_id: "queue-root".to_owned(),
                    started_at: MessageField::some(timestamp(1_711_469_199, 900_000_000)),
                    duration_ms: 1_248,
                    status: EnumOrUnknown::new(SpanStatus::Cancelled),
                    attributes: vec![
                        string_attribute("consumer.group", "inventory-staging"),
                        int_attribute("message.attempt", 5),
                    ],
                    events: vec![TraceEvent {
                        name: "dead-letter".to_owned(),
                        at: MessageField::some(timestamp(1_711_469_200, 2_000_000)),
                        fields: vec![
                            string_attribute("queue", "inventory.dlq"),
                            bool_attribute("requeued", false),
                        ],
                        special_fields: SpecialFields::default(),
                    }],
                    ..Default::default()
                },
                Span {
                    span_id: "queue-parse".to_owned(),
                    parent_span_id: Some("queue-root".to_owned()),
                    started_at: MessageField::some(timestamp(1_711_469_199, 950_000_000)),
                    duration_ms: 34,
                    status: EnumOrUnknown::new(SpanStatus::Ok),
                    attributes: vec![
                        string_attribute("format", "json"),
                        bytes_attribute("message.key", b"inventory:sku-4242"),
                    ],
                    events: vec![],
                    ..Default::default()
                },
            ],
            annotations: HashMap::from([
                ("owner".to_owned(), "supply-chain".to_owned()),
                ("rollout".to_owned(), "canary".to_owned()),
            ]),
            special_fields: SpecialFields::default(),
        }],
        labels: HashMap::from([
            ("cluster".to_owned(), "staging-1".to_owned()),
            ("source".to_owned(), "kafka".to_owned()),
        ]),
        user: MessageField::none(),
        alerts: vec![Alert {
            severity: EnumOrUnknown::new(Severity::Info),
            code: "MESSAGE_CANCELLED".to_owned(),
            related_span_ids: vec!["queue-root".to_owned()],
            summary: Some("staging queue message cancelled after retries".to_owned()),
            special_fields: SpecialFields::default(),
        }],
        transport: Some(TraceTransport::Queue(QueueMessage {
            topic: "inventory.events".to_owned(),
            partition: 7,
            offset: 98_771,
            headers: vec![
                header("tenant", "staging"),
                header("trace-export", "exp-queue-20260326"),
            ],
            special_fields: SpecialFields::default(),
        })),
        raw_envelope: b"trace-queue-envelope".to_vec(),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap()
}

fn trace_replay_request_sample() -> Vec<u8> {
    TraceReplayRequest {
        request_id: "replay-20260326-eu".to_owned(),
        target_environment: EnumOrUnknown::new(Environment::Development),
        focus_span_ids: vec![
            "span-root".to_owned(),
            "span-child-http".to_owned(),
            "span-worker".to_owned(),
        ],
        field_overrides: HashMap::from([
            ("labels.cluster".to_owned(), "dev-sandbox".to_owned()),
            (
                "transport.http.headers.x-debug-replay".to_owned(),
                "true".to_owned(),
            ),
        ]),
        expected_alerts: vec![
            Alert {
                severity: EnumOrUnknown::new(Severity::Critical),
                code: "HTTP_500_BURST".to_owned(),
                related_span_ids: vec!["span-root".to_owned()],
                summary: Some("replay should preserve checkout failure".to_owned()),
                special_fields: SpecialFields::default(),
            },
            Alert {
                severity: EnumOrUnknown::new(Severity::Warning),
                code: "WORKER_TIMEOUT".to_owned(),
                related_span_ids: vec!["span-worker".to_owned()],
                summary: Some("billing worker timeout should remain visible".to_owned()),
                special_fields: SpecialFields::default(),
            },
        ],
        metadata: vec![
            header("requested-by", "protobug"),
            header("ticket", "DBG-2026"),
            header("priority", "high"),
        ],
        source: Some(ReplaySource::TraceBundle(replay_trace_bundle())),
        checksum: b"sha256:9f7d0c2c7d8b".to_vec(),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap()
}

fn write_sample(name: &str, bytes: Vec<u8>) {
    fs::write(
        format!("{}/samples/{name}.bin", env!("CARGO_MANIFEST_DIR")),
        &bytes,
    )
    .unwrap();

    fs::write(
        format!("{}/samples/{name}.hex", env!("CARGO_MANIFEST_DIR")),
        format!("{}\n", hex::encode(&bytes)),
    )
    .unwrap();

    fs::write(
        format!("{}/samples/{name}.base64", env!("CARGO_MANIFEST_DIR")),
        format!("{}\n", BASE64_STANDARD.encode(&bytes)),
    )
    .unwrap();
}

fn replay_trace_bundle() -> TraceBundle {
    TraceBundle {
        export_id: "exp-replay-source".to_owned(),
        captured_at: MessageField::some(timestamp(1_711_465_600, 111_111_111)),
        environment: EnumOrUnknown::new(Environment::Production),
        services: vec![Service {
            name: "checkout-replay".to_owned(),
            version: "2026.03.26-replay".to_owned(),
            spans: vec![
                Span {
                    span_id: "span-root".to_owned(),
                    started_at: MessageField::some(timestamp(1_711_465_500, 123_000_000)),
                    duration_ms: 201,
                    status: EnumOrUnknown::new(SpanStatus::Error),
                    attributes: vec![
                        string_attribute("component", "replay"),
                        string_attribute("scenario", "checkout-timeout"),
                    ],
                    events: vec![TraceEvent {
                        name: "replayed".to_owned(),
                        at: MessageField::some(timestamp(1_711_465_500, 200_000_000)),
                        fields: vec![bool_attribute("dry_run", false)],
                        special_fields: SpecialFields::default(),
                    }],
                    ..Default::default()
                },
                Span {
                    span_id: "span-child-http".to_owned(),
                    parent_span_id: Some("span-root".to_owned()),
                    started_at: MessageField::some(timestamp(1_711_465_500, 150_000_000)),
                    duration_ms: 88,
                    status: EnumOrUnknown::new(SpanStatus::Ok),
                    attributes: vec![string_attribute("route", "/api/orders")],
                    events: vec![],
                    ..Default::default()
                },
                Span {
                    span_id: "span-worker".to_owned(),
                    parent_span_id: Some("span-root".to_owned()),
                    started_at: MessageField::some(timestamp(1_711_465_500, 250_000_000)),
                    duration_ms: 420,
                    status: EnumOrUnknown::new(SpanStatus::Timeout),
                    attributes: vec![string_attribute("queue", "billing-jobs")],
                    events: vec![],
                    ..Default::default()
                },
            ],
            annotations: HashMap::from([("profile".to_owned(), "replay".to_owned())]),
            special_fields: SpecialFields::default(),
        }],
        labels: HashMap::from([
            ("cluster".to_owned(), "prod-eu-1".to_owned()),
            ("purpose".to_owned(), "replay".to_owned()),
        ]),
        user: MessageField::some(UserContext {
            user_id: "replayer".to_owned(),
            roles: vec!["debugger".to_owned()],
            traits: HashMap::from([("tool".to_owned(), "protobug".to_owned())]),
            special_fields: SpecialFields::default(),
        }),
        alerts: vec![Alert {
            severity: EnumOrUnknown::new(Severity::Critical),
            code: "HTTP_500_BURST".to_owned(),
            related_span_ids: vec!["span-root".to_owned()],
            summary: Some("baseline replay source".to_owned()),
            special_fields: SpecialFields::default(),
        }],
        transport: Some(TraceTransport::Http(HttpRequest {
            method: "POST".to_owned(),
            path: "/replay".to_owned(),
            headers: vec![
                header("content-type", "application/x-protobuf"),
                header("x-replay", "true"),
            ],
            body: b"{\"mode\":\"replay\"}".to_vec(),
            special_fields: SpecialFields::default(),
        })),
        raw_envelope: b"trace-replay-source".to_vec(),
        special_fields: SpecialFields::default(),
    }
}

fn timestamp(seconds: i64, nanos: i32) -> Timestamp {
    Timestamp {
        seconds,
        nanos,
        special_fields: SpecialFields::default(),
    }
}

fn header(name: &str, value: &str) -> Header {
    Header {
        name: name.to_owned(),
        value: value.to_owned(),
        special_fields: SpecialFields::default(),
    }
}

fn string_attribute(key: &str, value: &str) -> Attribute {
    Attribute {
        key: key.to_owned(),
        value: Some(AttributeValue::StringValue(value.to_owned())),
        special_fields: SpecialFields::default(),
    }
}

fn int_attribute(key: &str, value: i64) -> Attribute {
    Attribute {
        key: key.to_owned(),
        value: Some(AttributeValue::IntValue(value)),
        special_fields: SpecialFields::default(),
    }
}

fn bool_attribute(key: &str, value: bool) -> Attribute {
    Attribute {
        key: key.to_owned(),
        value: Some(AttributeValue::BoolValue(value)),
        special_fields: SpecialFields::default(),
    }
}

fn bytes_attribute(key: &str, value: &[u8]) -> Attribute {
    Attribute {
        key: key.to_owned(),
        value: Some(AttributeValue::BytesValue(value.to_vec())),
        special_fields: SpecialFields::default(),
    }
}
