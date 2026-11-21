wit_bindgen::generate!({
    path: "../../../wit",
    world: "worker",
    generate_all,
});

use exports::ferropod::platform::queue_consumer::{
    Guest as QueueGuest, JobOutcome, QueueJob, RetryDirective,
};
use exports::ferropod::platform::scheduled::{CronEvent, Guest as ScheduledGuest};
use exports::wasi::http::incoming_handler::Guest as HttpGuest;
use ferropod::platform::{bucket, db, env, kv, log, queue};
use wasi::http::outgoing_handler;
use wasi::http::types::{
    Fields, IncomingBody, IncomingRequest, Method, OutgoingBody, OutgoingRequest, OutgoingResponse,
    ResponseOutparam, Scheme,
};

struct TestWorker;

export!(TestWorker);

fn send_response(out: ResponseOutparam, status: u16, content_type: &str, body: &[u8]) {
    let headers = Fields::new();
    let _ = headers.set("content-type", &[content_type.as_bytes().to_vec()]);
    let resp = OutgoingResponse::new(headers);
    let _ = resp.set_status_code(status);
    let out_body = resp.body().expect("outgoing response body");
    {
        let stream = out_body.write().expect("outgoing stream");
        for chunk in body.chunks(4096) {
            stream
                .blocking_write_and_flush(chunk)
                .expect("write response chunk");
        }
    }
    OutgoingBody::finish(out_body, None).expect("finish outgoing body");
    ResponseOutparam::set(out, Ok(resp));
}

fn handle_bindings() -> Result<String, String> {
    println!("worker stdout line");
    eprintln!("worker stderr line");
    log::emit(
        log::Level::Info,
        "wit log emitted",
        Some("{\"source\":\"guest\"}"),
    );

    // KV CRUD + atomic + cursor pagination
    kv::put("KV", "k1", b"v1", Some("m1"), None, false)?;
    kv::put("KV", "k2", b"v2", None, None, false)?;
    kv::put("KV", "k3", b"v3", None, None, false)?;
    let absent_inserted = kv::put_if_absent("KV", "k1", b"nope", None, None)?;
    let counter = kv::incr("KV", "counter", 5, None)?;
    let entry = kv::get_entry("KV", "k1")?.ok_or("missing k1")?;
    let kp1 = kv::list("KV", Some("k"), None, 2)?;
    let kp2 = kv::list("KV", Some("k"), kp1.cursor.as_deref(), 2)?;

    // DB execute + typed query
    db::execute(
        "DB",
        "CREATE TABLE IF NOT EXISTS items (id INTEGER PRIMARY KEY, name TEXT, active INTEGER)",
        &[],
    )?;
    db::execute(
        "DB",
        "INSERT OR REPLACE INTO items (id, name, active) VALUES (?1, ?2, ?3)",
        &[
            db::SqlValue::Integer(1),
            db::SqlValue::Text("alpha".into()),
            db::SqlValue::Boolean(true),
        ],
    )?;
    let qr = db::query(
        "DB",
        "SELECT id, name, active FROM items WHERE id = ?1",
        &[db::SqlValue::Integer(1)],
    )?;
    let db_name = match &qr.rows[0].values[1] {
        db::SqlValue::Text(s) => s.clone(),
        other => format!("{other:?}"),
    };

    // Queue enqueue
    let job_id = queue::enqueue(
        "QUEUE",
        b"from-worker",
        Some(&queue::EnqueueOptions {
            delay_secs: None,
            priority: Some(3),
            dedup_key: Some("w-dedup-1".into()),
            max_attempts: Some(4),
        }),
    )?;

    // Bucket put/get/list cursor/presign
    bucket::put("BUCKET", "a/1.txt", b"one", Some("text/plain"), Some("cm1"))?;
    bucket::put("BUCKET", "a/2.txt", b"two", Some("text/plain"), None)?;
    bucket::put("BUCKET", "a/3.txt", b"three", Some("text/plain"), None)?;
    let b_got = bucket::get("BUCKET", "a/1.txt")?.ok_or("missing bucket object")?;
    let bp1 = bucket::list("BUCKET", Some("a/"), None, 2)?;
    let bp2 = bucket::list("BUCKET", Some("a/"), bp1.cursor.as_deref(), 2)?;
    let presigned = bucket::presign_url("BUCKET", "a/1.txt", "get", 60)?;

    // Env + Secret
    let env_val = env::get("APP_MODE").unwrap_or_default();
    let secret_val = env::secret("API_SECRET").unwrap_or_default();

    Ok(format!(
        "{{\"kv_val\":\"{}\",\"kv_meta\":\"{}\",\"absent\":{},\"counter\":{},\"kv_pages\":[{},{}],\"db_cols\":{},\"db_name\":\"{}\",\"job_id\":{},\"bucket_val\":\"{}\",\"bucket_pages\":[{},{}],\"presigned_ok\":{},\"env\":\"{}\",\"secret\":\"{}\"}}",
        String::from_utf8_lossy(&entry.value),
        entry.metadata.unwrap_or_default(),
        absent_inserted,
        counter,
        kp1.keys.len(),
        kp2.keys.len(),
        qr.columns.len(),
        db_name,
        job_id,
        String::from_utf8_lossy(&b_got),
        bp1.objects.len(),
        bp2.objects.len(),
        presigned.contains("sig="),
        env_val,
        secret_val,
    ))
}

fn handle_outbound(authority: &str) -> Result<String, String> {
    let headers = Fields::new();
    let req = OutgoingRequest::new(headers);
    req.set_method(&Method::Get)
        .map_err(|_| "set_method failed")?;
    req.set_scheme(Some(&Scheme::Http))
        .map_err(|_| "set_scheme failed")?;
    req.set_authority(Some(authority))
        .map_err(|_| "set_authority failed")?;
    req.set_path_with_query(Some("/upstream"))
        .map_err(|_| "set_path failed")?;

    let fut = outgoing_handler::handle(req, None).map_err(|e| format!("outgoing handle: {e:?}"))?;
    fut.subscribe().block();
    let resp = fut
        .get()
        .ok_or("future missing")?
        .map_err(|_| "future consumed")?
        .map_err(|e| format!("http err: {e:?}"))?;

    let status = resp.status();
    let body = resp.consume().map_err(|_| "consume body failed")?;
    let stream = body.stream().map_err(|_| "body stream failed")?;
    let mut buf = Vec::new();
    loop {
        match stream.blocking_read(4096) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => buf.extend_from_slice(&chunk),
            Err(_) => break,
        }
    }
    drop(stream);
    IncomingBody::finish(body);

    Ok(format!("{status}:{}", String::from_utf8_lossy(&buf)))
}

impl HttpGuest for TestWorker {
    fn handle(request: IncomingRequest, response_out: ResponseOutparam) {
        let path = request.path_with_query().unwrap_or_else(|| "/".to_string());
        let path_only = path.split('?').next().unwrap_or("/");

        if path_only == "/bindings" {
            match handle_bindings() {
                Ok(json) => send_response(response_out, 200, "application/json", json.as_bytes()),
                Err(e) => send_response(response_out, 500, "text/plain", e.as_bytes()),
            }
            return;
        }

        if path_only == "/outbound" {
            let headers = request.headers();
            let vals = headers.get("x-upstream-authority");
            let auth = vals
                .first()
                .map(|v| String::from_utf8_lossy(v).into_owned())
                .unwrap_or_default();
            match handle_outbound(&auth) {
                Ok(txt) => send_response(response_out, 200, "text/plain", txt.as_bytes()),
                Err(e) => send_response(response_out, 502, "text/plain", e.as_bytes()),
            }
            return;
        }

        if path_only == "/loop" {
            loop {
                std::hint::black_box(1u64);
            }
        }

        if path_only == "/oom" {
            let size = std::hint::black_box(64 * 1024 * 1024usize);
            let mut v: Vec<u8> = vec![std::hint::black_box(1u8); size];
            std::hint::black_box(&mut v);
            send_response(response_out, 200, "text/plain", &[v[size - 1]]);
            return;
        }

        if path_only == "/api/data" {
            send_response(response_out, 200, "application/json", b"{\"ok\":true}");
            return;
        }

        if path_only == "/api/missing" {
            send_response(
                response_out,
                404,
                "application/json",
                b"{\"error\":\"not found\"}",
            );
            return;
        }

        if path_only == "/dashboard" {
            send_response(response_out, 404, "text/plain", b"not found");
            return;
        }

        let method = format!("{:?}", request.method());
        let reply = format!("ok:{method}:{path}");
        send_response(response_out, 200, "text/plain", reply.as_bytes());
    }
}

impl QueueGuest for TestWorker {
    fn handle_batch(_queue: String, jobs: Vec<QueueJob>) -> Vec<JobOutcome> {
        jobs.into_iter()
            .map(|j| {
                let text = String::from_utf8_lossy(&j.payload);
                match text.as_ref() {
                    "ack" => {
                        let _ = kv::put("KV", "queue_acked", b"yes", None, None, false);
                        JobOutcome::Ack
                    }
                    "retry" => JobOutcome::Retry(RetryDirective {
                        delay_secs: 1,
                        error: "retry requested".into(),
                    }),
                    "dead" => JobOutcome::Dead("dead requested".into()),
                    _ => JobOutcome::Ack,
                }
            })
            .collect()
    }
}

impl ScheduledGuest for TestWorker {
    fn handle_cron(event: CronEvent) -> Result<(), String> {
        kv::put(
            "KV",
            "cron_last_name",
            event.name.as_bytes(),
            None,
            None,
            false,
        )?;
        kv::put(
            "KV",
            "cron_last_target",
            event.target.as_bytes(),
            None,
            None,
            false,
        )?;
        log::emit(
            log::Level::Info,
            &format!("cron executed: {}", event.name),
            None,
        );
        Ok(())
    }
}
