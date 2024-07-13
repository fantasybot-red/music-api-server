mod helper;
mod subfile;

use std::{collections::HashMap, process::exit};

use axum::{
    body::Body, extract::{Path, Query, Request}, middleware::{self, Next}, response::{IntoResponse, Response}, routing::get, Router
};
use helper::{get_audio_subfile, strip_jsonc_comments, ConfigFile};
use librespot::{core::{cache::Cache, Session, SessionConfig, SpotifyId}, discovery::Credentials, metadata::{self, audio::AudioFiles, Metadata as _}};
use tokio::sync::Mutex;
use tokio_util::io::ReaderStream;
use lazy_static::lazy_static;

lazy_static! {
    static ref CONFIG: Mutex<Option<ConfigFile>> = Mutex::new(None);
    static ref sp_session: Mutex<Session> = Mutex::new(session_create());
}

fn session_create() -> Session {
    let session_config: SessionConfig = SessionConfig::default();
    let cache_config: Cache = Cache::new(Some("./"), None, None, Some(0)).unwrap();
    Session::new(session_config, Some(cache_config))
}

async fn load_config() -> ConfigFile {
    let config_str = std::fs::read_to_string("./config.json").unwrap();
    let clean_json = strip_jsonc_comments(&config_str, true);
    let config: ConfigFile = serde_json::from_str(&clean_json).unwrap();
    *CONFIG.lock().await = Some(config.clone());
    config
}

async fn login_spotify(session: Session) -> Session {
    if !session.is_invalid() && !session.username().is_empty() {
        return session;
    }
    let session = session_create();
    let sp_config = CONFIG.lock().await.clone().unwrap();
    let config = sp_config.spotify.unwrap();
    let username = config.username;
    let password = config.password;
    let credentials = Credentials::with_password(username, password);
    if let Err(e) = session.connect(credentials, true).await {
        eprintln!("Spotify Error connecting: {}", e);
        exit(1);
    }
    let cl_se = session.clone();
    *sp_session.lock().await = session;
    cl_se
}

async fn get_spsession() -> Session {
    let lock_sp = sp_session.lock().await;
    let session = lock_sp.to_owned();
    drop(lock_sp);
    login_spotify(session).await
}

async fn auth_middleware(
    request: Request,
    next: Next,
) -> Response {
    let config_lock = CONFIG.lock().await;
    let config = config_lock.to_owned();
    drop(config_lock);
    let config = config.unwrap();
    if request.uri().path().starts_with("/api/v1/") && config.api_key.is_some() {
        let api_key = config.api_key.unwrap();
        let mut header_check = false;
        let h_auth_r = request.headers().get("Authorization");
        if h_auth_r.is_some() {
            header_check = h_auth_r.unwrap().to_str().unwrap() == api_key;
        }
        let parsed_url = url::Url::parse(format!("http://localhost{}", request.uri().to_string()).as_str()).unwrap();
        let hash_query: HashMap<String, String> = parsed_url.query_pairs().into_owned().collect();
        let query_check = hash_query.get("api_key").unwrap_or(&"".to_string()) == &api_key;
        if !(header_check || query_check) {
            return Response::builder().status(401).body(Body::from("Unauthorized")).unwrap();
        }
    }
    let response = next.run(request).await;
    response
}

#[tokio::main]
async fn main() {
    let config_file = load_config().await;
    if config_file.spotify.is_some() {
        println!("Connecting Spotify...");
        let session = get_spsession().await;
        println!("Connected Spotify as {}", session.username());
    }
    let app = Router::new()
        .route("/", get(|| async { "We like your mom :)" }))
        .route("/api/v1/sp/audio/:sp_uri", get(sp_audio))
        .route("/api/v1/sp/metadata/:sp_uri", get(sp_metadata))
        .route("/api/v1/sp/search", get(sp_search))
        .layer(middleware::from_fn(auth_middleware));


    let addr = config_file.bind;
    let listener = tokio::net::TcpListener::bind(addr.clone()).await.unwrap();
    println!("Start server at {}", addr);
    let _ = axum::serve(listener, app).await;
}

async fn sp_audio(Path(sp_uri): Path<String>) -> impl IntoResponse {
    if sp_session.lock().await.username().is_empty() {
        return Response::builder().status(501).body(Body::from("Spotify is disable")).unwrap();
    }
    let sspclient = get_spsession().await;
    let subfile_r = get_audio_subfile(&sspclient, &sp_uri).await;
    if subfile_r.is_none() {
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    let subfile = subfile_r.unwrap();
    let file_format = subfile.format;
    let stream = ReaderStream::new(subfile);
    let mut content_type = "audio/aac";
    if AudioFiles::is_mp3(file_format) {
        content_type = "audio/mp3"
    } else if AudioFiles::is_ogg_vorbis(file_format) {
        content_type = "audio/ogg"
    } else if AudioFiles::is_flac(file_format) {
        content_type = "audio/flac"
    }
    Response::builder().header("Content-Type", content_type).body(Body::from_stream(stream)).unwrap()
}

async fn sp_metadata(Path(sp_uri): Path<String>) -> impl IntoResponse {
    if sp_session.lock().await.username().is_empty() {
        return Response::builder().status(501).body(Body::from("Spotify is disable")).unwrap();
    }
    let sspclient = get_spsession().await;
    let sp_id_r = SpotifyId::from_uri(&sp_uri);
    if sp_id_r.is_err() {
        eprintln!("{:?}", sp_id_r.unwrap_err());
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    let sp_id = sp_id_r.unwrap();
    let j_str = match sp_id.item_type {
        librespot::core::spotify_id::SpotifyItemType::Album => {
            let mut r_obj;
            loop {
                r_obj = metadata::Album::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        librespot::core::spotify_id::SpotifyItemType::Artist => {
            let mut r_obj;
            loop {
                r_obj = metadata::Artist::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        librespot::core::spotify_id::SpotifyItemType::Episode => {
            let mut r_obj;
            loop {
                r_obj = metadata::Episode::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        librespot::core::spotify_id::SpotifyItemType::Playlist => {
            let mut r_obj;
            loop {
                r_obj = metadata::Playlist::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        librespot::core::spotify_id::SpotifyItemType::Show => {
            let mut r_obj;
            loop {
                r_obj = metadata::Show::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        librespot::core::spotify_id::SpotifyItemType::Track => {
            let mut r_obj;
            loop {
                r_obj = metadata::Track::get(&sspclient, &sp_id).await;
                if r_obj.is_err() {
                    let err = r_obj.unwrap_err();
                    if err.kind == librespot::core::error::ErrorKind::ResourceExhausted {
                        continue;
                    }
                    eprintln!("{:?}", err);
                    return Response::builder().status(500).body(Body::empty()).unwrap();
                }
                break;
            }
            Some(serde_json::to_string(&r_obj.unwrap()))
        },
        _ => None
    };
    if j_str.is_none() {
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    let j_str_r = j_str.unwrap();
    if j_str_r.is_err() {
        eprintln!("{:?}", j_str_r.unwrap_err());
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    Response::builder().header("Content-Type", "application/json").body(Body::from(j_str_r.unwrap())).unwrap()
}

async fn sp_search(Query(params): Query<HashMap<String, String>>) -> impl IntoResponse {
    if sp_session.lock().await.username().is_empty() {
        return Response::builder().status(501).body(Body::from("Spotify is disable")).unwrap();
    }
    let sspclient = get_spsession().await;
    let q_r = params.get("q");
    if q_r.is_none() {
        return Response::builder().status(406).body(Body::empty()).unwrap();
    }
    let q = q_r.unwrap();
    let mut url_obj = url::Url::parse("hm://searchview/km/v4/search").unwrap();
    url_obj.path_segments_mut().unwrap().push(q);
    url_obj.query_pairs_mut()
    .append_pair("entityVersion", "2")
    .append_pair("limit", "20")
    .append_pair("username", &sspclient.username())
    .append_pair("locale", "")
    .append_pair("country", &sspclient.country())
    .append_pair("catalogue", "");
    let url = url_obj.to_string();
    let data_r = sspclient.mercury().get(&url);
    if data_r.is_err() {
        eprintln!("Mercury error ?");
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    let data = data_r.unwrap();
    let data_obj_r = data.await;
    if data_obj_r.is_err() {
        eprintln!("{:?}", data_obj_r.unwrap_err());
        return Response::builder().status(500).body(Body::empty()).unwrap();
    }
    let data_obj = data_obj_r.unwrap().payload.clone();
    let inter_vec = data_obj[0].to_owned();
    return Response::builder().header("Content-Type", "application/json").body(Body::from(inter_vec)).unwrap();
}
