use anyhow::Ok;
use esp_idf_svc::{http::{headers::content_type, server::EspHttpServer}, io::{Read, Write}};

const MAX_PAYLOAD_LEN: usize = 128;

pub struct HttpServer {
    esp_http_server: EspHttpServer<'static>,
}

impl HttpServer {
    pub fn new() -> Self {
        let mut server = EspHttpServer::new(&esp_idf_svc::http::server::Configuration {
            ..Default::default()
        })
        .unwrap();


        Self {
            esp_http_server: server
        }
    }
    pub fn get<S: AsRef<str>, F: Fn() -> Response + Send + Sync + 'static>(&mut self, url: S, handler: F) -> &mut Self {
        self.esp_http_server
            .fn_handler(url.as_ref(), esp_idf_svc::http::Method::Get, move |request| {
                let response = handler();
                request
                    .into_response(
                        response.status_code,
                        None,
                        &[content_type(&response.content_type)],
                    )?
                    .write(response.body.as_bytes())
                    .map(|_| ())
            })
            .unwrap();

        self
    }

    pub fn post<S: AsRef<str>, B: for<'a> serde::Deserialize<'a> + 'static, F: Fn(B) -> Response + Send + Sync + 'static>(&mut self, url: S, handler: F) -> &mut Self {
        self.esp_http_server
            .fn_handler::<anyhow::Error, _>(url.as_ref(), esp_idf_svc::http::Method::Post, move |mut request| {
                let len = request.header("Content-Length").unwrap_or("0").parse::<usize>()?;

                if len > MAX_PAYLOAD_LEN {
                    request.into_status_response(413)?
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
                    .write(response.body.as_bytes())?;
                    Ok(())
            })
            .unwrap();

        self
    }
}

pub struct Response {
    status_code: u16,
    content_type: String,
    body: String,
}

impl Response {
    pub fn ok() -> Self {
        Self {
            body: "".to_string(),
            content_type: "application/json".to_string(),
            status_code: 200
        }
    }
}

pub struct Json(String);

impl Into<Response> for Json {
    fn into(self) -> Response {
        Response {
            status_code: 200,
            content_type: "application/json".to_string(),
            body: self.0,
        }
    }
}
