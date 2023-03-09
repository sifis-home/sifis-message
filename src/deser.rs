pub(crate) mod method {
    use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

    pub(crate) fn serialize<S>(method: &reqwest::Method, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        method.as_str().serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D>(deserializer: D) -> Result<reqwest::Method, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw_method = String::deserialize(deserializer)?;
        raw_method
            .parse()
            .map_err(|err| de::Error::custom(format_args!("invalid method: {err}")))
    }

    #[cfg(test)]
    mod tests {
        use serde_json::json;

        use super::*;

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct A(#[serde(with = "super")] reqwest::Method);

        #[test]
        fn serialize() {
            assert_eq!(
                serde_json::to_value(A(reqwest::Method::GET)).unwrap(),
                json!("GET"),
            );
        }

        #[test]
        fn deserialize() {
            assert_eq!(
                serde_json::from_value::<A>(json!("POST")).unwrap(),
                A(reqwest::Method::POST),
            );
        }
    }
}

pub(crate) mod headers {
    use std::fmt;

    use reqwest::header::{HeaderName, HeaderValue};
    use serde::{
        de::{self, SeqAccess, Visitor},
        ser::SerializeSeq,
        Deserialize, Deserializer, Serializer,
    };

    pub(crate) fn serialize<S>(
        headers: &Vec<(HeaderName, HeaderValue)>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut seq = serializer.serialize_seq(Some(headers.len()))?;
        for (name, value) in headers {
            let name = name.as_str();
            match value.to_str() {
                Ok(value) => seq.serialize_element(&(name, value))?,
                Err(_) => seq.serialize_element(&(name, value.as_bytes()))?,
            }
        }
        seq.end()
    }

    pub(crate) fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<Vec<(HeaderName, HeaderValue)>, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawValue {
            String(String),
            Bytes(Vec<u8>),
        }

        struct HeadersVisitor;

        impl<'de> Visitor<'de> for HeadersVisitor {
            type Value = Vec<(HeaderName, HeaderValue)>;

            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("a sequence of pairs header name and header value")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                let mut out = seq.size_hint().map_or_else(Vec::new, Vec::with_capacity);
                while let Some((name, value)) = seq.next_element::<(String, RawValue)>()? {
                    let name = name.parse().map_err(|err| {
                        de::Error::custom(format_args!("header name is invalid: {err}"))
                    })?;
                    let value = match value {
                        RawValue::String(s) => HeaderValue::from_str(&s),
                        RawValue::Bytes(b) => HeaderValue::from_bytes(&b),
                    }
                    .map_err(|err| {
                        de::Error::custom(format_args!("header value is invalid: {err}"))
                    })?;

                    out.push((name, value));
                }

                Ok(out)
            }
        }

        deserializer.deserialize_seq(HeadersVisitor)
    }

    #[cfg(test)]
    mod tests {
        use serde::Serialize;
        use serde_json::json;

        use super::*;

        #[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
        struct A(#[serde(with = "super")] Vec<(HeaderName, HeaderValue)>);

        #[test]
        fn serialize() {
            assert_eq!(
                serde_json::to_value(A(vec![
                    (
                        HeaderName::from_static("hello"),
                        HeaderValue::from_str("world").unwrap()
                    ),
                    (
                        HeaderName::from_static("test"),
                        HeaderValue::from_bytes(&[32, 33, 255]).unwrap(),
                    )
                ]))
                .unwrap(),
                json!([["hello", "world"], ["test", [32, 33, 255]]])
            );
        }

        #[test]
        fn deserialize() {
            assert_eq!(
                serde_json::from_value::<A>(json!([["hello", "world"], ["test", [32, 33, 255]]]))
                    .unwrap(),
                A(vec![
                    (
                        HeaderName::from_static("hello"),
                        HeaderValue::from_str("world").unwrap()
                    ),
                    (
                        HeaderName::from_static("test"),
                        HeaderValue::from_bytes(&[32, 33, 255]).unwrap(),
                    )
                ]),
            );
        }
    }
}
