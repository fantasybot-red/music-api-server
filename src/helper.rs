
use librespot::{audio::{AudioDecrypt, AudioFile}, core::{Session, SpotifyId}, metadata::audio::{AudioFileFormat, AudioFiles, AudioItem}};
use serde::{Deserialize, Serialize};



use crate::subfile::{self, Subfile};

pub fn stream_data_rate(format: AudioFileFormat) -> usize {
    let kbps = match format {
        AudioFileFormat::OGG_VORBIS_96 => 12,
        AudioFileFormat::OGG_VORBIS_160 => 20,
        AudioFileFormat::OGG_VORBIS_320 => 40,
        AudioFileFormat::MP3_256 => 32,
        AudioFileFormat::MP3_320 => 40,
        AudioFileFormat::MP3_160 => 20,
        AudioFileFormat::MP3_96 => 12,
        AudioFileFormat::MP3_160_ENC => 20,
        AudioFileFormat::AAC_24 => 3,
        AudioFileFormat::AAC_48 => 6,
        AudioFileFormat::FLAC_FLAC => 112, // assume 900 kbit/s on average
    };
    kbps * 1024
}

pub async fn get_audio_subfile(session: &Session, uri: &str) -> Option<subfile::Subfile<AudioDecrypt<AudioFile>>> {
    let track_r = SpotifyId::from_uri(uri);
    if track_r.is_err() {
        return None;
    }
    let track = track_r.unwrap();
    let audio_r = AudioItem::get_file(&session, track).await;
    if audio_r.is_err() {
        return None;
    }
    let audio = audio_r.unwrap();
    let audio_files_key = audio.files.keys().filter(|x| AudioFiles::is_mp3(**x) || AudioFiles::is_ogg_vorbis(**x) || AudioFiles::is_flac(**x));
    for i in audio_files_key {
        let file_id = audio.files.get(i).unwrap();
        let key_r = session.audio_key().request(track, *file_id).await;
        if key_r.is_err() {
            continue;
        }
        let key = key_r.unwrap();
        let bytes_per_second = stream_data_rate(*i);
        let encrypted_file_r = AudioFile::open(&session, *file_id, bytes_per_second)
            .await;
        if encrypted_file_r.is_err() {
            continue;
        }
        let encrypted_file = encrypted_file_r.unwrap();
        let length = encrypted_file.get_stream_loader_controller().unwrap().len() as u64;
        let decrypted_file = AudioDecrypt::new(Some(key), encrypted_file);  
        
        let subfile_r = Subfile::new(
            decrypted_file,
            length,
            *i
        );
        
        if subfile_r.is_err() {
            continue;
        }
        let subfile = subfile_r.unwrap();
        return Some(subfile);
    }
    None
}

pub fn strip_jsonc_comments(jsonc_input: &str, preserve_locations: bool) -> String {
    let mut json_output = String::new();

    let mut block_comment_depth: u8 = 0;
    let mut is_in_string: bool = false; // Comments cannot be in strings

    for line in jsonc_input.split('\n') {
        let mut last_char: Option<char> = None;
        for cur_char in line.chars() {
            // Check whether we're in a string
            if block_comment_depth == 0 && last_char != Some('\\') && cur_char == '"' {
                is_in_string = !is_in_string;
            }

            // Check for line comment start
            if !is_in_string && last_char == Some('/') && cur_char == '/' {
                last_char = None;
                if preserve_locations {
                    json_output.push_str("  ");
                }
                break; // Stop outputting or parsing this line
            }
            // Check for block comment start
            if !is_in_string && last_char == Some('/') && cur_char == '*' {
                block_comment_depth += 1;
                last_char = None;
                if preserve_locations {
                    json_output.push_str("  ");
                }
            // Check for block comment end
            } else if !is_in_string && last_char == Some('*') && cur_char == '/' {
                if block_comment_depth > 0 {
                    block_comment_depth -= 1;
                }
                last_char = None;
                if preserve_locations {
                    json_output.push_str("  ");
                }
            // Output last char if not in any block comment
            } else {
                if block_comment_depth == 0 {
                    if let Some(last_char) = last_char {
                        json_output.push(last_char);
                    }
                } else {
                    if preserve_locations {
                        json_output.push_str(" ");
                    }
                }
                last_char = Some(cur_char);
            }
        }

        // Add last char and newline if not in any block comment
        if let Some(last_char) = last_char {
            if block_comment_depth == 0 {
                json_output.push(last_char);
            } else if preserve_locations {
                json_output.push(' ');
            }
        }

        // Remove trailing whitespace from line
        while json_output.ends_with(' ') {
            json_output.pop();
        }
        json_output.push('\n');
    }

    json_output
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigFile {
    pub bind: String,
    pub api_key: Option<String>,
    pub spotify: Option<SpotifyConfig>
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpotifyConfig {
    pub username: String,
    pub password: String
}


