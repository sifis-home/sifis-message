mod attribute;
mod data_type;

use std::{
    fmt::{self, Display},
    io,
};

use serde::{Deserialize, Serialize};

use self::{attribute::Attribute, data_type::DataType};

pub trait Requestable {
    const ACTION_ID: &'static str;
    const ATTRIBUTE_ID: &'static str;
    const HAS_DATA: bool;

    type Type<'a>: DataType + Serialize
    where
        Self: 'a;

    #[inline]
    fn value(&self) -> Self::Type<'_> {
        unimplemented!("Requestable::value should be implemented for types with HAS_DATA = true")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(bound = "T: Requestable")]
pub struct Request<'a, T> {
    #[serde(rename = "@ReturnPolicyIdList")]
    return_policy_id_list: bool,

    #[serde(rename = "@CombinedDecision")]
    combined_decision: bool,

    #[serde(rename = "@xmlns")]
    xmlns: Namespace,

    #[serde(rename = "Attributes")]
    attributes: [Attribute<'a, T>; 3],
}

impl<'a, T> Request<'a, T> {
    #[inline]
    const fn new(subject_id: &'a str, resource_id: &'a str, action: T) -> Self {
        Self {
            return_policy_id_list: false,
            combined_decision: false,
            xmlns: Namespace::Wd17,
            attributes: [
                Attribute::Subject(subject_id),
                Attribute::Resource(resource_id),
                Attribute::Action(action),
            ],
        }
    }
}

impl<'a, T> Request<'a, T>
where
    T: Requestable,
{
    pub fn write_xml<W>(&self, mut writer: W) -> Result<(), WriteXmlError>
    where
        W: io::Write,
    {
        writer
            .write_all(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#)
            .map_err(WriteXmlError::Io)?;
        let mut adapter = FmtAdapter {
            writer,
            error: None,
        };
        if let Err(err) = quick_xml::se::to_writer(&mut adapter, self) {
            let err = if let Some(err) = adapter.error.take() {
                WriteXmlError::Io(err)
            } else {
                WriteXmlError::Xml(err)
            };

            return Err(err);
        }
        Ok(())
    }
}

#[derive(Debug)]
struct FmtAdapter<T> {
    writer: T,
    error: Option<io::Error>,
}

impl<T> fmt::Write for FmtAdapter<T>
where
    T: io::Write,
{
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.writer.write_all(s.as_bytes()).map_err(|err| {
            self.error = Some(err);
            fmt::Error
        })
    }
}

#[derive(Debug)]
pub enum WriteXmlError {
    Io(io::Error),
    Xml(quick_xml::DeError),
}

impl Display for WriteXmlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WriteXmlError::Io(_) => f.write_str("I/O error while writing XML for Request"),
            WriteXmlError::Xml(_) => f.write_str("unable to serialize Request to XML"),
        }
    }
}

impl std::error::Error for WriteXmlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WriteXmlError::Io(source) => Some(source),
            WriteXmlError::Xml(source) => Some(source),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Namespace {
    #[serde(rename = "urn:oasis:names:tc:xacml:3.0:core:schema:wd-17")]
    Wd17,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct DummyDataType;

impl DataType for DummyDataType {
    const URI: &'static str = "";
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GetStatus;

impl Requestable for GetStatus {
    const ACTION_ID: &'static str = "get_status";
    const ATTRIBUTE_ID: &'static str = "";
    const HAS_DATA: bool = false;

    type Type<'a> = DummyDataType
    where
        Self: 'a;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct GetBrightness;

impl Requestable for GetBrightness {
    const ACTION_ID: &'static str = "get_brightness";
    const ATTRIBUTE_ID: &'static str = "";
    const HAS_DATA: bool = false;

    type Type<'a> = DummyDataType
    where
        Self: 'a;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SetBrightness(pub u8);

impl Requestable for SetBrightness {
    const ACTION_ID: &'static str = "set_brightness";
    const ATTRIBUTE_ID: &'static str = "eu:sifis-home:1.0:action:brightness-value";
    const HAS_DATA: bool = true;

    type Type<'a> = u8
    where
        Self: 'a;

    fn value(&self) -> Self::Type<'_> {
        self.0
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TurnOff;

impl Requestable for TurnOff {
    const ACTION_ID: &'static str = "turn_off";
    const ATTRIBUTE_ID: &'static str = "";
    const HAS_DATA: bool = false;

    type Type<'a> = DummyDataType
    where
        Self: 'a;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct TurnOn;

impl Requestable for TurnOn {
    const ACTION_ID: &'static str = "turn_on";
    const ATTRIBUTE_ID: &'static str = "";
    const HAS_DATA: bool = false;

    type Type<'a> = DummyDataType
    where
        Self: 'a;
}

#[inline]
pub const fn get_status<'a>(subject_id: &'a str, resource_id: &'a str) -> Request<'a, GetStatus> {
    Request::new(subject_id, resource_id, GetStatus)
}

#[inline]
pub const fn get_brightness<'a>(
    subject_id: &'a str,
    resource_id: &'a str,
) -> Request<'a, GetBrightness> {
    Request::new(subject_id, resource_id, GetBrightness)
}

#[inline]
pub const fn set_brightness<'a>(
    subject_id: &'a str,
    resource_id: &'a str,
    value: u8,
) -> Request<'a, SetBrightness> {
    Request::new(subject_id, resource_id, SetBrightness(value))
}

#[inline]
pub const fn turn_on<'a>(subject_id: &'a str, resource_id: &'a str) -> Request<'a, TurnOn> {
    Request::new(subject_id, resource_id, TurnOn)
}

#[inline]
pub const fn turn_off<'a>(subject_id: &'a str, resource_id: &'a str) -> Request<'a, TurnOff> {
    Request::new(subject_id, resource_id, TurnOff)
}

#[cfg(test)]
mod tests {
    use crate::ucs::request::set_brightness;

    use super::get_status;

    #[test]
    fn write_action_with_no_data() {
        let mut out = Vec::new();
        get_status("subject", "resource")
            .write_xml(&mut out)
            .unwrap();

        assert_eq!(
            out, b"\
            <?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <Request ReturnPolicyIdList=\"false\" CombinedDecision=\"false\" \
                     xmlns=\"urn:oasis:names:tc:xacml:3.0:core:schema:wd-17\">\
                <Attributes Category=\"urn:oasis:names:tc:xacml:1.0:subject-category:access-subject\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:subject:subject-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">subject</AttributeValue>\
                    </Attribute>\
                </Attributes>\
                <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:resource\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:resource:resource-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">resource</AttributeValue>\
                    </Attribute>\
                </Attributes>\
                <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:action\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:action:action-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">get_status</AttributeValue>\
                    </Attribute>\
                </Attributes>\
            </Request>\
            ",
        );
    }

    #[test]
    fn write_action_with_data() {
        let mut out = Vec::new();
        set_brightness("subject", "resource", 42)
            .write_xml(&mut out)
            .unwrap();

        assert_eq!(
            out, b"\
            <?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
            <Request ReturnPolicyIdList=\"false\" CombinedDecision=\"false\" \
                     xmlns=\"urn:oasis:names:tc:xacml:3.0:core:schema:wd-17\">\
                <Attributes Category=\"urn:oasis:names:tc:xacml:1.0:subject-category:access-subject\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:subject:subject-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">subject</AttributeValue>\
                    </Attribute>\
                </Attributes>\
                <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:resource\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:resource:resource-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">resource</AttributeValue>\
                    </Attribute>\
                </Attributes>\
                <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:action\">\
                    <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:action:action-id\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">set_brightness</AttributeValue>\
                    </Attribute>\
                    <Attribute AttributeId=\"eu:sifis-home:1.0:action:brightness-value\" IncludeInResult=\"false\">\
                        <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#integer\">42</AttributeValue>\
                    </Attribute>\
                </Attributes>\
            </Request>\
            ",
        );
    }
}
