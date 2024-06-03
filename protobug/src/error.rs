use derive_more::{Display, Error};

#[derive(Debug, Display, Error)]
#[display(fmt = "Schema validation failed")]
pub(crate) struct Validate;

#[derive(Debug, Display, Error)]
#[display(fmt = "Inspecting message failed")]
pub(crate) struct Inspect;

#[derive(Debug, Display, Error)]
#[display(fmt = "Invalid Protobuf schema")]
pub(crate) struct InvalidSchema;

#[derive(Debug, Display, Error)]
#[display(fmt = "No top-level messages found")]
pub(crate) struct NoTopLevelMessages;

#[derive(Debug, Display, Error)]
#[display(fmt = "Schemas with multiple top-level messages are not currently supported")]
pub(crate) struct MultipleTopLevelMessages;
