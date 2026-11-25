//! The Worker around the statements: `POST /v1/day` takes a report of at most 64 KiB, checks it
//! with `accept` against today's UTC date, and writes its rows to D1 with the date it arrived.
//! Nothing else of the request (address, headers, time of day) is kept; the address only keys
//! the rate limit.

use brain_report::{MAX_BODY, accept};
use futures_util::StreamExt;
use worker::wasm_bindgen::JsValue;
use worker::{Context, Date, Env, Method, Request, Response, Result, event};

use crate::{Param, limit_key, statements, status, utc_date};

#[event(fetch)]
async fn fetch(mut req: Request, env: Env, _ctx: Context) -> Result<Response> {
    if req.method() != Method::Post || req.path() != "/v1/day" {
        return Ok(Response::empty()?.with_status(404));
    }
    let address = req.headers().get("cf-connecting-ip")?.unwrap_or_default();
    let limit = env.rate_limiter("USAGE_LIMIT")?;
    if !limit.limit(limit_key(&address)).await?.success {
        return Ok(Response::empty()?.with_status(429));
    }
    let mut body = Vec::new();
    // A request with no body has no stream; accept refuses the empty body as malformed.
    if let Ok(mut stream) = req.stream() {
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if body.len() + chunk.len() > MAX_BODY {
                return Ok(Response::empty()?.with_status(413));
            }
            body.extend_from_slice(&chunk);
        }
    }
    let signature = req.headers().get("brain-signature")?.unwrap_or_default();
    // jiff's own clock panics on wasm32-unknown-unknown, so the date comes from the runtime.
    let today = utc_date(Date::now().as_millis() as i64)
        .ok_or_else(|| worker::Error::RustError("the runtime's clock is out of range".into()))?;
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
