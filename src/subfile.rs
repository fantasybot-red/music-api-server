use std::{io::{self, Read, Seek, SeekFrom}, pin::Pin, task::{Context, Poll}};

use librespot::metadata::audio::{AudioFileFormat, AudioFiles};
use tokio::{io::{AsyncRead, ReadBuf}, task::block_in_place};

const SPOTIFY_OGG_HEADER_END: u64 = 0xa7;

pub struct Subfile<T: Read + Seek> {
    stream: T,
    offset: u64,
    length: u64,
    pub format: AudioFileFormat
}

impl<T: Read + Seek> Subfile<T> {
    pub fn new(mut stream: T, length: u64, format: AudioFileFormat) -> Result<Subfile<T>, io::Error> {

        let is_ogg_vorbis = AudioFiles::is_ogg_vorbis(format);
        let offset = if is_ogg_vorbis { SPOTIFY_OGG_HEADER_END } else { 0 };
        let target = SeekFrom::Start(offset);
        stream.seek(target)?;
        Ok(Subfile {
            stream,
            offset,
            length,
            format
        })
    }
}

impl<T: Read + Seek> Read for Subfile<T> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.stream.read(buf)
    }
}

impl<T: Read + Seek> Seek for Subfile<T> {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let pos = match pos {
            SeekFrom::Start(offset) => SeekFrom::Start(offset + self.offset),
            SeekFrom::End(offset) => {
                if (self.length as i64 - offset) < self.offset as i64 {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "newpos would be < self.offset",
                    ));
                }
                pos
            }
            _ => pos,
        };

        let newpos = self.stream.seek(pos)?;
        Ok(newpos - self.offset)
    }
}

impl<T: Read + Seek + Unpin> AsyncRead for Subfile<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();

        // Use `block_in_place` to perform blocking I/O in an async context
        block_in_place(|| {
            let buffer = buf.initialize_unfilled();
            let n = this.stream.read(buffer)?;
            buf.advance(n);
            Poll::Ready(Ok(()))
        })
    }
}