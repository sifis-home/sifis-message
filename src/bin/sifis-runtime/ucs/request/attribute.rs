use serde::{Serialize, Serializer};

use crate::ucs::request::DataType;

use super::Requestable;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Attribute<'a, T> {
    Subject(&'a str),
    Resource(&'a str),
    Action(T),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename = "Attributes")]
struct SimpleAttributesRepr<'a> {
    #[serde(rename = "@Category")]
    category: &'a str,

    #[serde(rename = "Attribute")]
    attribute: AttributeRepr<'a, &'a str>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename = "Attributes")]
struct AttributesRepr<'a, T: Requestable + 'a> {
    #[serde(rename = "@Category")]
    category: &'a str,

    #[serde(rename = "Attribute")]
    attribute: (AttributeRepr<'a, &'a str>, AttributeRepr<'a, T::Type<'a>>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename = "Attribute")]
struct AttributeRepr<'a, T> {
    #[serde(rename = "@AttributeId")]
    attribute_id: &'a str,

    #[serde(rename = "@IncludeInResult")]
    include_in_result: bool,

    #[serde(rename = "AttributeValue")]
    value: AttributeValueRepr<'a, T>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename = "AttributeValue")]
struct AttributeValueRepr<'a, T> {
    #[serde(rename = "@DataType")]
    data_type: &'a str,

    #[serde(rename = "$text")]
    content: T,
}

const ACCESS_SUBJECT_CATEGORY: &str =
    "urn:oasis:names:tc:xacml:1.0:subject-category:access-subject";
const RESOURCE_CATEGORY: &str = "urn:oasis:names:tc:xacml:3.0:attribute-category:resource";
const ACTION_CATEGORY: &str = "urn:oasis:names:tc:xacml:3.0:attribute-category:action";

const SUBJECT_ID_ATTRIBUTE: &str = "urn:oasis:names:tc:xacml:1.0:subject:subject-id";
const RESOURCE_ID_ATTRIBUTE: &str = "urn:oasis:names:tc:xacml:1.0:resource:resource-id";
const ACTION_ID_ATTRIBUTE: &str = "urn:oasis:names:tc:xacml:1.0:action:action-id";

const STRING_DATA_TYPE: &str = "http://www.w3.org/2001/XMLSchema#string";

impl<'a, T> Serialize for Attribute<'a, T>
where
    T: Requestable,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Attribute::Subject(subject) => SimpleAttributesRepr {
                category: ACCESS_SUBJECT_CATEGORY,
                attribute: AttributeRepr {
                    attribute_id: SUBJECT_ID_ATTRIBUTE,
                    include_in_result: false,
                    value: AttributeValueRepr {
                        data_type: STRING_DATA_TYPE,
                        content: subject,
                    },
                },
            }
            .serialize(serializer),
            Attribute::Resource(resource) => SimpleAttributesRepr {
                category: RESOURCE_CATEGORY,
                attribute: AttributeRepr {
                    attribute_id: RESOURCE_ID_ATTRIBUTE,
                    include_in_result: false,
                    value: AttributeValueRepr {
                        data_type: STRING_DATA_TYPE,
                        content: resource,
                    },
                },
            }
            .serialize(serializer),
            Attribute::Action(action) => {
                if T::HAS_DATA {
                    AttributesRepr::<T> {
                        category: ACTION_CATEGORY,
                        attribute: (
                            AttributeRepr {
                                attribute_id: ACTION_ID_ATTRIBUTE,
                                include_in_result: false,
                                value: AttributeValueRepr {
                                    data_type: STRING_DATA_TYPE,
                                    content: T::ACTION_ID,
                                },
                            },
                            AttributeRepr {
                                attribute_id: T::ATTRIBUTE_ID,
                                include_in_result: false,
                                value: AttributeValueRepr {
                                    data_type: T::Type::URI,
                                    content: action.value(),
                                },
                            },
                        ),
                    }
                    .serialize(serializer)
                } else {
                    SimpleAttributesRepr {
                        category: ACTION_CATEGORY,
                        attribute: AttributeRepr {
                            attribute_id: ACTION_ID_ATTRIBUTE,
                            include_in_result: false,
                            value: AttributeValueRepr {
                                data_type: STRING_DATA_TYPE,
                                content: T::ACTION_ID,
                            },
                        },
                    }
                    .serialize(serializer)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ucs::request::{DummyDataType, Requestable};

    use super::Attribute;

    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub struct DummyRequestable;

    impl Requestable for DummyRequestable {
        const ACTION_ID: &'static str = "dummy";
        const ATTRIBUTE_ID: &'static str = "";
        const HAS_DATA: bool = false;

        type Type<'a> = DummyDataType
    where
        Self: 'a;
    }

    #[test]
    fn serialize_subject() {
        let serialized =
            quick_xml::se::to_string(&Attribute::<DummyRequestable>::Subject("my subject"))
                .unwrap();
        assert_eq!(
            "\
            <Attributes Category=\"urn:oasis:names:tc:xacml:1.0:subject-category:access-subject\">\
                <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:subject:subject-id\" IncludeInResult=\"false\">\
                    <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">my subject</AttributeValue>\
                </Attribute>\
            </Attributes>\
            ",
            serialized,
        );
    }

    #[test]
    fn serialize_resource() {
        let serialized =
            quick_xml::se::to_string(&Attribute::<DummyRequestable>::Resource("my resource"))
                .unwrap();
        assert_eq!(
            "\
            <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:resource\">\
                <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:resource:resource-id\" IncludeInResult=\"false\">\
                    <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">my resource</AttributeValue>\
                </Attribute>\
            </Attributes>\
            ",
            serialized,
        );
    }

    #[test]
    fn serialize_action_no_data() {
        let serialized = quick_xml::se::to_string(&Attribute::Action(DummyRequestable)).unwrap();
        assert_eq!(
            "\
            <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:action\">\
                <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:action:action-id\" IncludeInResult=\"false\">\
                    <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">dummy</AttributeValue>\
                </Attribute>\
            </Attributes>\
            ",
            serialized,
        );
    }

    #[test]
    fn serialize_action_with_data() {
        let serialized = quick_xml::se::to_string(&Attribute::Action(DummyRequestable)).unwrap();
        assert_eq!(
            "\
            <Attributes Category=\"urn:oasis:names:tc:xacml:3.0:attribute-category:action\">\
                <Attribute AttributeId=\"urn:oasis:names:tc:xacml:1.0:action:action-id\" IncludeInResult=\"false\">\
                    <AttributeValue DataType=\"http://www.w3.org/2001/XMLSchema#string\">dummy</AttributeValue>\
                </Attribute>\
            </Attributes>\
            ",
            serialized,
        );
    }
}
