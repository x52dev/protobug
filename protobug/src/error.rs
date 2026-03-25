use derive_more::derive::{Display, Error};

#[derive(Debug, Display, Error)]
#[display("Inspecting message failed")]
pub struct Inspect;

#[derive(Debug, Display, Error)]
#[display("Invalid Protobuf schema")]
pub struct InvalidSchema;

#[derive(Debug, Display, Error)]
#[display("No top-level messages found")]
pub struct NoTopLevelMessages;

#[derive(Debug, Display, Error)]
#[display("Schemas with multiple top-level messages are not currently supported")]
pub struct MultipleTopLevelMessages;
