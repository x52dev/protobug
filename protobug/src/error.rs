use derive_more::derive::{Display, Error};

#[derive(Debug, Display, Error)]
#[display("Schema validation failed")]
pub(crate) struct Validate;

#[derive(Debug, Display, Error)]
#[display("Inspecting message failed")]
pub(crate) struct Inspect;

#[derive(Debug, Display, Error)]
#[display("Invalid Protobuf schema")]
pub(crate) struct InvalidSchema;

#[derive(Debug, Display, Error)]
#[display("No top-level messages found")]
pub(crate) struct NoTopLevelMessages;

#[derive(Debug, Display, Error)]
#[display("Schemas with multiple top-level messages are not currently supported")]
pub(crate) struct MultipleTopLevelMessages;
