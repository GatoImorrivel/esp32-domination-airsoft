use anyhow::Ok;
use esp_idf_svc::{
    http::{
        headers::content_type,
        server::EspHttpServer,
    },
    io::{Read, Write}
};
use include_dir::{include_dir, Dir};

static WEB_BUILD: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web/dist");

const MAX_PAYLOAD_LEN: usize = 128;

pub struct HttpServer {
    esp_http_server: EspHttpServer<'static>,
}

impl HttpServer {
    pub fn new() -> Self {
        let server = EspHttpServer::new(&esp_idf_svc::http::server::Configuration {
            ..Default::default()
        })
        .unwrap();

        Self {
            esp_http_server: server,
        }
    }
    pub fn get<S: AsRef<str>, F: Fn() -> Response + Send + Sync + 'static>(
        &mut self,
        url: S,
        handler: F,
    ) -> &mut Self {
        self.esp_http_server
            .fn_handler(
                url.as_ref(),
                esp_idf_svc::http::Method::Get,
                move |request| {
                    let response = handler();
                    let body = response.body();
                    request
                        .into_response(
                            response.status_code,
                            None,
                            &[
                                content_type(&response.content_type.into_media_type().0),
                            ],
                        )?
                        .write_all(body)
                        .map(|_| ())
                },
            )
            .unwrap();

        self
    }

    pub fn post<
        S: AsRef<str>,
        B: for<'a> serde::Deserialize<'a> + 'static,
        F: Fn(B) -> Response + Send + Sync + 'static,
    >(
        &mut self,
        url: S,
        handler: F,
    ) -> &mut Self {
        self.esp_http_server
            .fn_handler::<anyhow::Error, _>(
                url.as_ref(),
                esp_idf_svc::http::Method::Post,
                move |mut request| {
                    let len = request
                        .header("Content-Length")
                        .unwrap_or("0")
                        .parse::<usize>()?;

                    if len > MAX_PAYLOAD_LEN {
                        request
                            .into_status_response(413)?
                            .write_all("Request too big".as_bytes())?;
                        return Ok(());
                    }

                    let mut buf = vec![0; len];
                    request.read_exact(&mut buf)?;

                    let response = handler(serde_json::from_slice::<B>(&buf)?);
                    request
                        .into_response(
                            response.status_code,
                            None,
                            &[content_type(&response.content_type.into_media_type().0)],
                        )?
                        .write_all(response.body())?;
                    Ok(())
                },
            )
            .unwrap();

        self
    }
}

pub enum ResponseBody {
    String(String),
    StaticString(&'static str),
    Bytes(&'static [u8]),
}

pub struct Response {
    status_code: u16,
    content_type: ContentType,
    body: ResponseBody,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            body: ResponseBody::StaticString(""),
            content_type: ContentType::Text,
            status_code: 200,
        }
    }

    pub fn body(&self) -> &[u8] {
        match &self.body {
            ResponseBody::StaticString(payload) => payload.as_bytes(),
            ResponseBody::String(payload) => payload.as_bytes(),
            ResponseBody::Bytes(payload) => payload,
        }
    }
}

pub struct Json(String);

impl Into<Response> for Json {
    fn into(self) -> Response {
        Response {
            status_code: 200,
            content_type: ContentType::Json,
            body: ResponseBody::String(self.0),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MediaType(&'static str);

#[derive(Debug, Clone, Copy)]
pub enum ContentType {
    Js,
    Css,
    Html,
    Svg,
    Png,
    Jpg,
    Ico,
    Woff,
    Woff2,
    Ttf,
    Json,
    OctetStream,
    Text
}

impl ContentType {
    pub fn from_file_extension<S: AsRef<str>>(extension: S) -> Self {
        match extension.as_ref() {
            "js" => Self::Js,
            "mjs" => Self::Js,
            "css" => Self::Css,
            "html" => Self::Html,
            "svg" => Self::Svg,
            "png" => Self::Png,
            "jpg" | "jpeg" => Self::Jpg,
            "ico" => Self::Ico,
            "woff" => Self::Woff,
            "woff2" => Self::Woff2,
            "ttf" => Self::Ttf,
            "json" => Self::Json,
            "txt" => Self::Text,
            _ => Self::OctetStream,
        }
    }

    pub fn into_media_type(&self) -> MediaType {
        let media_type = match self {
            Self::Js => "application/javascript",
            Self::Css => "text/css",
            Self::Html => "text/html",
            Self::Svg => "image/svg+xml",
            Self::Png => "image/png",
            Self::Jpg => "image/jpeg",
            Self::Ico => "image/x-icon",
            Self::Woff => "font/woff",
            Self::Woff2 => "font/woff2",
            Self::Ttf => "font/ttf",
            Self::Json => "application/json",
            Self::OctetStream => "application/octet-stream",
            Self::Text => "text/plain"
        };
        MediaType(media_type)
    }
}

impl<S: AsRef<str>> From<S> for ContentType {
    fn from(value: S) -> Self {
        Self::from_file_extension(value)
    }
}

impl Into<MediaType> for ContentType {
    fn into(self) -> MediaType {
        self.into_media_type()
    }
}

impl Into<&'static str> for ContentType {
    fn into(self) -> &'static str {
        self.into_media_type().0
    }
}

pub fn load_web(server: &mut HttpServer) {
    if let Some(index) = WEB_BUILD.get_file("index.html") {
        let contents = index.contents();
        server.get("/", move || Response {
            status_code: 200,
            content_type: ContentType::Html,
            body: ResponseBody::Bytes(contents),
        });
    }

    fn register_dir(dir: &Dir<'static>, server: &mut HttpServer) {
        for file in dir.files() {
            // The file path relative to the root of `dist/`
            let route = format!("/{}", file.path().display());

            let contents = file.contents();
            let extension = file.path().extension().and_then(|s| s.to_str()).unwrap_or("");
            let content_type = ContentType::from(extension);

            let contents = contents;

            server.get(route, move || Response {
                status_code: 200,
                content_type: content_type,
                body: ResponseBody::Bytes(contents),
            });
        }

        for subdir in dir.dirs() {
            register_dir(subdir, server);
        }
    }

    register_dir(&WEB_BUILD, server);
}
