//! The Worker around the statements: `POST /v1/day` takes a report of at most 64 KiB, checks it
//! with `accept` against today's UTC date, and writes its rows to D1. Nothing else of the request
//! (address, headers, timing) is kept.

use brain_report::accept;
use futures_util::StreamExt;
use jiff::Timestamp;
use jiff::tz::TimeZone;
use worker::wasm_bindgen::JsValue;
use worker::{Context, Date, Env, Method, Request, Response, Result, event};

use crate::{Param, statements, status};

const MAX_BODY: usize = 65_536;

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Post || req.path() != "/v1/day" {
        return Ok(Response::empty()?.with_status(404));
    }
    let mut body = Vec::new();
    // A request with no body has no stream; accept refuses the empty body as malformed.
    if let Ok(mut stream) = req.stream() {
        while let Some(chunk) = stream.next().await {
            body.extend_from_slice(&chunk?);
            if body.len() > MAX_BODY {
                return Ok(Response::empty()?.with_status(413));
            }
        }
    }
    let signature = req.headers().get("brain-signature")?.unwrap_or_default();
    // jiff's own clock panics on wasm32-unknown-unknown, so the date comes from the runtime.
    let today = Timestamp::from_millisecond(Date::now().as_millis() as i64)
        .map_err(|e| worker::Error::RustError(e.to_string()))?
        .to_zoned(TimeZone::UTC)
        .date();
    let report = match accept(&body, &signature, today) {
        Ok(report) => report,
        Err(refused) => return Ok(Response::empty()?.with_status(status(&refused))),
    };
    let db = env.d1("USAGE_DB")?;
    let batch = statements(&report, today)
        .into_iter()
        .map(|s| {
            let params: Vec<JsValue> = s
                .params
                .iter()
                .map(|p| match p {
                    Param::Text(t) => JsValue::from_str(t),
                    Param::Int(i) => JsValue::from_f64(*i as f64),
                })
                .collect();
            db.prepare(s.sql).bind(&params)
        })
        .collect::<Result<Vec<_>>>()?;
    db.batch(batch).await?;
    Ok(Response::empty()?.with_status(204))
}
