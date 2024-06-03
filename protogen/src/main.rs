use std::fs;

use base64::prelude::*;
use protobuf::{
    well_known_types::timestamp::Timestamp, EnumOrUnknown, Message as _, MessageField,
    SpecialFields,
};
use protogen::system_event::{system_event, ClickButton, ClickEvent, SystemEvent};

fn main() {
    let ev = SystemEvent {
        timestamp: MessageField::some(Timestamp::UNIX_EPOCH),
        reason: Some("user clicked".to_owned()),
        event: Some(system_event::Event::Click(ClickEvent {
            button: EnumOrUnknown::new(ClickButton::Left),
            x: 69,
            y: 420,
            ..Default::default()
        })),
        special_fields: SpecialFields::default(),
    }
    .write_to_bytes()
    .unwrap();

    fs::write(
        concat![env!("CARGO_MANIFEST_DIR"), "/samples/system-event.bin"],
        &ev,
    )
    .unwrap();

    fs::write(
        concat![env!("CARGO_MANIFEST_DIR"), "/samples/system-event.hex"],
        hex::encode(&ev),
    )
    .unwrap();

    fs::write(
        concat![env!("CARGO_MANIFEST_DIR"), "/samples/system-event.base64"],
        BASE64_STANDARD.encode(&ev),
    )
    .unwrap();
}
