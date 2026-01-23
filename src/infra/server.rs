use anyhow::Ok;
use esp_idf_svc::{
    http::{headers::content_type, server::EspHttpServer},
    io::{Read, Write},
};
use include_dir::{Dir, include_dir};

static SVELTE_BUILD: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/web-ui/dist");

const MAX_PAYLOAD_LEN: usize = 128;

pub fn load_svelte(server: &mut HttpServer) {
    // Serve index.html at `/` as the main entrypoint
    if let Some(index) = SVELTE_BUILD.get_file("index.html") {
        let contents = index.contents();
        server.get("/", move || Response {
            status_code: 200,
            content_type: "text/html".into(),
            body: ResponseBody::Bytes(contents),
        });
    }

    // Recursively register all files in the dist folder
    fn register_dir(dir: &Dir<'static>, server: &mut HttpServer) {
        for file in dir.files() {
            // The file path relative to the root of `dist/`
            let route = format!("/{}", file.path().display());

            let contents = file.contents();
            let content_type = match file.path().extension().and_then(|s| s.to_str()) {
                Some("js") => "application/javascript",
                Some("mjs") => "application/javascript",
                Some("css") => "text/css",
                Some("html") => "text/html",
                Some("svg") => "image/svg+xml",
                Some("png") => "image/png",
                Some("jpg") | Some("jpeg") => "image/jpeg",
                Some("ico") => "image/x-icon",
                Some("woff") => "font/woff",
                Some("woff2") => "font/woff2",
                Some("ttf") => "font/ttf",
                Some("json") => "application/json",
                _ => "application/octet-stream",
            }
            .to_string();

            // Move into closure
            let contents = contents;
            let content_type = content_type.clone();

            server.get(route, move || Response {
                status_code: 200,
                content_type: content_type.clone(),
                body: ResponseBody::Bytes(contents),
            });
        }

        // Recurse into subdirectories
        for subdir in dir.dirs() {
            register_dir(subdir, server);
        }
    }

    register_dir(&SVELTE_BUILD, server);
}

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
                    request
                        .into_response(
                            response.status_code,
                            None,
                            &[content_type(&response.content_type)],
                        )?
                        .write(response.body())
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
                            &[content_type(&response.content_type)],
                        )?
                        .write(response.body())?;
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
    Bytes(&'static [u8])
}

pub struct Response {
    status_code: u16,
    content_type: String,
    body: ResponseBody,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            body: ResponseBody::StaticString(""),
            content_type: "application/json".to_string(),
            status_code: 200,
        }
    }

    pub fn body(&self) -> &[u8] {
        match &self.body {
            ResponseBody::StaticString(payload) => {
                payload.as_bytes()
            },
            ResponseBody::String(payload) => {
                payload.as_bytes()
            },
            ResponseBody::Bytes(payload) => {
                payload
            }
        }
    }
}

pub struct Json(String);

impl Into<Response> for Json {
    fn into(self) -> Response {
        Response {
            status_code: 200,
            content_type: "application/json".to_string(),
            body: ResponseBody::String(self.0),
        }
    }
}
