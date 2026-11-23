//! AWS Signature Version 4 for one request with a body and the headers `content-type`, `host` and `x-amz-date`
//! (the EC2 Query API's). The secret only keys the HMACs; it is never sent.

use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256};

/// The Query API's content type.
pub const FORM: &str = "application/x-www-form-urlencoded; charset=utf-8";

/// What a request is signed over.
pub struct Signing<'a> {
    pub method: &'a str,
    pub host: &'a str,
    /// The URI path, already encoded (`/`).
    pub path: &'a str,
    /// The canonical query string (its parameters sorted and encoded), empty for none.
    pub query: &'a str,
    pub content_type: &'a str,
    pub body: &'a str,
    pub key_id: &'a str,
    pub secret: &'a str,
    pub region: &'a str,
    pub service: &'a str,
    /// The signing time, in ms.
    pub now: i64,
}

/// The headers a signed request carries.
pub struct Signed {
    /// `x-amz-date`: `20261123T120000Z`.
    pub amz_date: String,
    /// `authorization`.
    pub authorization: String,
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("HMAC takes any key length");
    m.update(data.as_bytes());
    m.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &str) -> String {
    hex::encode(Sha256::digest(data.as_bytes()))
}

/// The key that signs requests to `service` in `region` on `date` (`YYYYMMDD`).
pub fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> Vec<u8> {
    let k = hmac(format!("AWS4{secret}").as_bytes(), date);
    let k = hmac(&k, region);
    let k = hmac(&k, service);
    hmac(&k, "aws4_request")
}

/// The canonical request (step 1 of the signing).
pub fn canonical_request(s: &Signing<'_>, amz_date: &str) -> String {
    format!(
        "{}\n{}\n{}\ncontent-type:{}\nhost:{}\nx-amz-date:{amz_date}\n\ncontent-type;host;x-amz-date\n{}",
        s.method,
        s.path,
        s.query,
        s.content_type,
        s.host,
        sha256_hex(s.body)
    )
}

pub fn sign(s: &Signing<'_>) -> Signed {
    let t = jiff::Timestamp::from_millisecond(s.now).unwrap_or_default();
    let amz_date = t.strftime("%Y%m%dT%H%M%SZ").to_string();
    let date = &amz_date[..8];
    let scope = format!("{date}/{}/{}/aws4_request", s.region, s.service);
    let to_sign = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha256_hex(&canonical_request(s, &amz_date)));
    let signature = hex::encode(hmac(&signing_key(s.secret, date, s.region, s.service), &to_sign));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders=content-type;host;x-amz-date, Signature={signature}",
        s.key_id
    );
    Signed { amz_date, authorization }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS's own examples (the IAM ListUsers request of the Signature Version 4 docs, 2015-08-30T12:36:00Z).
    #[test]
    fn it_matches_the_examples_in_the_aws_docs() {
        let key = signing_key("wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "20120215", "us-east-1", "iam");
        assert_eq!(hex::encode(key), "f4780e2d9f65fa895f9c67b32ce1baf0b0d8a43505a000a1a9e090d414db404d");

        let s = Signing {
            method: "GET",
            host: "iam.amazonaws.com",
            path: "/",
            query: "Action=ListUsers&Version=2010-05-08",
            content_type: FORM,
            body: "",
            key_id: "AKIDEXAMPLE",
            secret: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            region: "us-east-1",
            service: "iam",
            now: 1_440_938_160_000,
        };
        assert_eq!(
            sha256_hex(&canonical_request(&s, "20150830T123600Z")),
            "f536975d06c0309214f805bb90ccff089219ecd68b2577efef23edd43b7e1a59"
        );
        let signed = sign(&s);
        assert_eq!(signed.amz_date, "20150830T123600Z");
        assert_eq!(
            signed.authorization,
            "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, \
             SignedHeaders=content-type;host;x-amz-date, \
             Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
        );
    }
}
