// Copyright 2018-2025 the Deno authors. MIT license.

use std::ops::Deref;

use deno_core::op2;
use digest::Digest;
use serde::Deserialize;
use x509_parser::der_parser::asn1_rs::Any;
use x509_parser::der_parser::asn1_rs::Tag;
use x509_parser::der_parser::oid::Oid;
pub use x509_parser::error::X509Error;
use x509_parser::extensions;
use x509_parser::pem;
use x509_parser::prelude::*;
use yoke::Yoke;
use yoke::Yokeable;

use super::KeyObjectHandle;

enum CertificateSources {
  Der(Box<[u8]>),
  Pem(pem::Pem),
}

#[derive(Yokeable)]
struct CertificateView<'a> {
  cert: X509Certificate<'a>,
}

pub(crate) struct Certificate {
  inner: Yoke<CertificateView<'static>, Box<CertificateSources>>,
}

impl deno_core::GarbageCollected for Certificate {}

impl Certificate {
  fn fingerprint_digest<D: Digest>(&self) -> Option<String> {
    if let CertificateSources::Pem(pem) = self.inner.backing_cart().as_ref() {
      let mut hasher = D::new();
      hasher.update(&pem.contents);
      let bytes = hasher.finalize();
      // OpenSSL returns colon separated upper case hex values.
      let mut hex = String::with_capacity(bytes.len() * 2);
      for byte in bytes {
        hex.push_str(&format!("{:02X}:", byte));
      }
      hex.pop();
      Some(hex)
    } else {
      None
    }
  }
}

impl<'a> Deref for CertificateView<'a> {
  type Target = X509Certificate<'a>;

  fn deref(&self) -> &Self::Target {
    &self.cert
  }
}

deno_error::js_error_wrapper!(X509Error, JsX509Error, "Error");

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum X509Subject {
  Always,
  Never,
}

impl Default for X509Subject {
  fn default() -> Self {
    X509Subject::Always
  }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct X509CheckOptions {
  subject: X509Subject,
  wildcards: bool,
  partial_wildcards: bool,
  multi_label_wildcards: bool,
  single_label_subdomains: bool,
}

impl Default for X509CheckOptions {
  fn default() -> Self {
    X509CheckOptions {
      subject: X509Subject::default(),
      wildcards: true,
      partial_wildcards: true,
      multi_label_wildcards: false,
      single_label_subdomains: false,
    }
  }
}

#[op2]
impl Certificate {
  #[constructor]
  #[cppgc]
  fn new(#[anybuffer] buf: &[u8]) -> Result<Certificate, JsX509Error> {
    let source = match pem::parse_x509_pem(buf) {
      Ok((_, pem)) => CertificateSources::Pem(pem),
      Err(_) => CertificateSources::Der(buf.to_vec().into_boxed_slice()),
    };

    let inner =
    Yoke::<CertificateView<'static>, Box<CertificateSources>>::try_attach_to_cart(
      Box::new(source),
      |source| {
        let cert = match source {
          CertificateSources::Pem(pem) => pem.parse_x509()?,
          CertificateSources::Der(buf) => {
            X509Certificate::from_der(buf).map(|(_, cert)| cert)?
          }
        };
        Ok::<_, X509Error>(CertificateView { cert })
      },
    )?;

    Ok(Certificate { inner })
  }

  #[getter]
  fn ca(&self) -> bool {
    let cert = self.inner.get().deref();
    cert.is_ca()
  }

  #[fast]
  fn check_email(&self, #[string] email: &str) -> bool {
    let cert = self.inner.get().deref();
    let subject = cert.subject();
    if subject
      .iter_email()
      .any(|e| e.as_str().unwrap_or("") == email)
    {
      return true;
    }

    let subject_alt = cert
      .extensions()
      .iter()
      .find(|e| {
        e.oid == x509_parser::oid_registry::OID_X509_EXT_SUBJECT_ALT_NAME
      })
      .and_then(|e| match e.parsed_extension() {
        extensions::ParsedExtension::SubjectAlternativeName(s) => Some(s),
        _ => None,
      });

    if let Some(subject_alt) = subject_alt {
      for name in &subject_alt.general_names {
        if let extensions::GeneralName::RFC822Name(n) = name {
          if *n == email {
            return true;
          }
        }
      }
    }

    false
  }

  #[fast]
  fn check_host(&self, #[string] name: &str) {}

  #[fast]
  #[rename("checkIP")]
  fn check_ip(&self, #[string] name: &str) {}

  #[fast]
  fn check_issued(&self, #[cppgc] issuer: &Certificate) -> bool {
    let cert = self.inner.get().deref();
    let issuer = issuer.inner.get().deref();

    cert.issuer() == issuer.subject()
  }

  #[fast]
  fn check_private_key(&self, #[cppgc] key: &KeyObjectHandle) -> bool {
    false
  }

  #[getter]
  #[string]
  fn fingerprint(&self) -> Option<String> {
    self.fingerprint_digest::<sha1::Sha1>()
  }

  #[getter]
  #[string]
  fn fingerprint256(&self) -> Option<String> {
    self.fingerprint_digest::<sha2::Sha256>()
  }

  #[getter]
  #[string]
  fn fingerprint512(&self) -> Option<String> {
    self.fingerprint_digest::<sha2::Sha512>()
  }

  #[getter]
  #[string]
  fn issuer(&self) -> Result<String, JsX509Error> {
    let cert = self.inner.get().deref();
    x509name_to_string(cert.issuer(), oid_registry()).map_err(Into::into)
  }

  #[getter]
  #[string]
  fn info_access(&self) -> Option<String> {
    let cert = self.inner.get().deref();
    let info_access = cert
      .extensions()
      .iter()
      .find(|e| {
        e.oid == x509_parser::oid_registry::OID_PKIX_AUTHORITY_INFO_ACCESS
      })
      .and_then(|e| match e.parsed_extension() {
        extensions::ParsedExtension::AuthorityInfoAccess(a) => Some(a),
        _ => None,
      });

    // info_access.map(|a| a.to_string())
    todo!()
  }

  #[getter]
  fn issuer_certificate(&self) {}

  #[getter]
  #[serde]
  fn key_usage(&self) -> Option<Vec<&'static str>> {
    let cert = self.inner.get().deref();
    let key_usage = cert
      .extensions()
      .iter()
      .find(|e| e.oid == x509_parser::oid_registry::OID_X509_EXT_KEY_USAGE)
      .and_then(|e| match e.parsed_extension() {
        extensions::ParsedExtension::KeyUsage(k) => Some(k),
        _ => None,
      });

    let flags = key_usage.map(|k| k.flags).unwrap_or(0);

    if flags == 0x0 {
      return None;
    }

    let mut res = Vec::new();
    if flags & 0x01 != 0 {
      res.push("Digital Signature");
    } else if flags & 0x02 != 0 {
      res.push("NonRepudiation");
    } else if flags & 0x04 != 0 {
      res.push("KeyEncipherment");
    } else if flags & 0x08 != 0 {
      res.push("DataEncipherment");
    } else if flags & 0x10 != 0 {
      res.push("KeyAgreement");
    } else if flags & 0x20 != 0 {
      res.push("KeyCert Sign");
    } else if flags & 0x40 != 0 {
      res.push("CRLSign");
    } else if flags & 0x80 != 0 {
      res.push("EncipherOnly");
    } else if flags & 0x100 != 0 {
      res.push("DecipherOnly");
    }

    Some(res)
  }

  #[getter]
  #[cppgc]
  fn public_key(
    &self,
  ) -> Result<KeyObjectHandle, super::keys::X509PublicKeyError> {
    let cert = self.inner.get().deref();
    let public_key = &cert.tbs_certificate.subject_pki;

    KeyObjectHandle::new_x509_public_key(public_key)
  }

  #[getter]
  #[string]
  fn serial_number(&self) -> String {
    let cert = self.inner.get().deref();
    let mut s = cert.serial.to_str_radix(16);
    s.make_ascii_uppercase();
    s
  }

  #[getter]
  fn subject_alt_name(&self) {}

  #[getter]
  #[string]
  fn subject(&self) -> Result<String, JsX509Error> {
    let cert = self.inner.get().deref();
    x509name_to_string(cert.subject(), oid_registry()).map_err(Into::into)
  }

  #[getter]
  #[string]
  fn valid_from(&self) -> String {
    let cert = self.inner.get().deref();
    cert.validity().not_before.to_string()
  }

  #[getter]
  #[string]
  fn valid_to(&self) -> String {
    let cert = self.inner.get().deref();
    cert.validity().not_after.to_string()
  }

  #[string]
  fn to_string(&self) -> String {
    todo!()
  }

  #[rename("toJSON")]
  #[string]
  fn to_json(&self) -> String {
    todo!()
  }

  #[fast]
  fn verify(&self) -> bool {
    todo!()
  }
}

// Attempt to convert attribute to string. If type is not a string, return value is the hex
// encoding of the attribute value
fn attribute_value_to_string(
  attr: &Any,
  _attr_type: &Oid,
) -> Result<String, X509Error> {
  // TODO: replace this with helper function, when it is added to asn1-rs
  match attr.tag() {
    Tag::NumericString
    | Tag::BmpString
    | Tag::VisibleString
    | Tag::PrintableString
    | Tag::GeneralString
    | Tag::ObjectDescriptor
    | Tag::GraphicString
    | Tag::T61String
    | Tag::VideotexString
    | Tag::Utf8String
    | Tag::Ia5String => {
      let s = core::str::from_utf8(attr.data)
        .map_err(|_| X509Error::InvalidAttributes)?;
      Ok(s.to_owned())
    }
    _ => {
      // type is not a string, get slice and convert it to base64
      Ok(data_encoding::HEXUPPER.encode(attr.as_bytes()))
    }
  }
}

fn x509name_to_string(
  name: &X509Name,
  oid_registry: &oid_registry::OidRegistry,
) -> Result<String, x509_parser::error::X509Error> {
  // Lifted from https://github.com/rusticata/x509-parser/blob/4d618c2ed6b1fc102df16797545895f7c67ee0fe/src/x509.rs#L543-L566
  // since it's a private function (Copyright 2017 Pierre Chifflier)
  name.iter_rdn().try_fold(String::new(), |acc, rdn| {
    rdn
      .iter()
      .try_fold(String::new(), |acc2, attr| {
        let val_str =
          attribute_value_to_string(attr.attr_value(), attr.attr_type())?;
        // look ABBREV, and if not found, use shortname
        let abbrev = match oid2abbrev(attr.attr_type(), oid_registry) {
          Ok(s) => String::from(s),
          _ => format!("{:?}", attr.attr_type()),
        };
        let rdn = format!("{}={}", abbrev, val_str);
        match acc2.len() {
          0 => Ok(rdn),
          _ => Ok(acc2 + " + " + rdn.as_str()),
        }
      })
      .map(|v| match acc.len() {
        0 => v,
        _ => acc + "\n" + v.as_str(),
      })
  })
}
