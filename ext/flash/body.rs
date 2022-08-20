// Copyright 2018-2022 the Deno authors. All rights reserved. MIT license.

use crate::chunked::Decoder as ChunkedDecoder;
use std::io::{self, Read};
use std::pin::Pin;
use tokio::sync::mpsc;

pub type Body = Box<[u8]>;

#[derive(PartialEq)]
pub enum Decoder {
  Fixed(FixedDecoder),
  Chunked(()),
  None,
}

#[derive(PartialEq)]
pub struct FixedDecoder {
  pub content_read: usize,
  pub content_length: usize,
}

pub struct BodyReader {
  pub read_tx: mpsc::Sender<Body>,
  pub read_rx: mpsc::Receiver<Body>,

  pub backing_buf: Box<[u8]>,
  pub decoder: Decoder,
  pub keep_alive: bool,
}

impl BodyReader {
  #[inline]
  pub fn new(keep_alive: bool, decoder: Decoder) -> BodyReader {
    let (read_tx, read_rx) = mpsc::channel(1);
    let mut backing_buf = vec![0; 16 * 1024 + 256].into_boxed_slice();
    BodyReader {
      read_tx,
      read_rx,
      keep_alive,
      decoder,
      backing_buf,
    }
  }

  pub fn step<R: Read>(&mut self, source: &mut R) {
    match self.decoder {
      Decoder::Fixed(FixedDecoder {
        content_length,
        mut content_read,
      }) => loop {
        if content_read >= content_length {
          self.decoder = Decoder::None;
          return;
        }

        match source.read(&mut self.backing_buf) {
          Ok(n) => {
            content_read += n;
            let _ = self.read_tx.blocking_send(self.backing_buf[..n].to_vec().into_boxed_slice());
          }
          _ => break,
        }
      },
      Decoder::Chunked(_decoder) => {}
      Decoder::None => {}
    }
  }

  pub async fn read(&mut self) -> io::Result<Body> {

    match self.read_rx.recv().await {
      Some(body) => Ok(body),
      None => Err(io::Error::new(io::ErrorKind::Other, "read error")),
    }
  }
}
