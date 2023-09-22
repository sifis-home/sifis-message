use std::io;

use base64::write::EncoderStringWriter;
use serde::{ser::SerializeMap, Serialize, Serializer};
use uuid::Uuid;

use crate::ucs::request;

use super::request::Requestable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TryAccessRequest {
    pub message_id: Uuid,
    pub request: Request,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub subject: String,
    pub resource: String,
    pub kind: RequestKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    GetStatus,
    TurnOn,
    TurnOff,
}

impl Serialize for TryAccessRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let Request {
            ref subject,
            ref resource,
            kind: request_kind,
        } = self.request;
        let mut request_encoder =
            EncoderStringWriter::new(&base64::engine::general_purpose::STANDARD);

        match request_kind {
            RequestKind::GetStatus => write_request_xml_no_data::<S, _>(
                &mut request_encoder,
                subject,
                resource,
                request::get_status,
            )?,
            RequestKind::TurnOn => write_request_xml_no_data::<S, _>(
                &mut request_encoder,
                subject,
                resource,
                request::turn_on,
            )?,
            RequestKind::TurnOff => write_request_xml_no_data::<S, _>(
                &mut request_encoder,
                subject,
                resource,
                request::turn_off,
            )?,
        };
        let request = request_encoder.into_inner();

        let mut map = serializer.serialize_map(Some(4))?;
        map.serialize_entry("purpose", &"TRY")?;
        map.serialize_entry("message_id", &self.message_id)?;
        map.serialize_entry("request", &request)?;
        map.serialize_entry("policy", &())?;
        map.end()
    }
}

fn write_request_xml_no_data<'a, S, T>(
    writer: impl io::Write,
    subject: &'a str,
    resource: &'a str,
    f: impl Fn(&'a str, &'a str) -> request::Request<'a, T>,
) -> Result<(), S::Error>
where
    S: Serializer,
    T: Requestable,
{
    f(subject, resource)
        .write_xml(writer)
        .map_err(|err| serde::ser::Error::custom(format!("unable to encode request: {err:?}")))
}

#[cfg(test)]
mod tests {
    use base64::Engine;
    use uuid::Uuid;

    use super::TryAccessRequest;

    #[test]
    fn serialize_try_access_request_without_value() {
        use serde_json::Value;

        let message_id = Uuid::new_v4();
        let request = TryAccessRequest {
            message_id,
            request: super::Request {
                subject: "subject".to_owned(),
                resource: "resource".to_owned(),
                kind: super::RequestKind::TurnOff,
            },
        };
        let request_json = serde_json::to_value(request).unwrap();
        assert_eq!(
            request_json.get("purpose").unwrap(),
            &Value::String("TRY".to_string()),
        );
        assert_eq!(
            request_json.get("message_id").unwrap(),
            &Value::String(message_id.to_string()),
        );
        assert_eq!(request_json.get("policy").unwrap(), &Value::Null);

        let encoded_request = request_json.get("request").unwrap().as_str().unwrap();
        let decoded_request = base64::engine::general_purpose::STANDARD
            .decode(encoded_request)
            .unwrap();
        let decoded_request = std::str::from_utf8(&decoded_request).unwrap();

        assert_eq!(
            decoded_request,
            "\
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
                            <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">turn_off</AttributeValue>\
                        </Attribute>\
                    </Attributes>\
                </Request>\
            ",
        );
    }
}
