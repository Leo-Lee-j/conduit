use super::*;

struct Fixture {
    host: Host,
    requests: async_channel::Receiver<Request>,
    agent: ureq::Agent,
}

impl Fixture {
    fn new() -> Self {
        let (host, requests) = Host::bind("", 0).unwrap();
        let agent = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(5)))
            .build()
            .into();
        Self {
            host,
            requests,
            agent,
        }
    }

    fn api(&self, path: &str) -> String {
        format!("{}/api/remote/v1{path}", self.host.origin())
    }

    fn pair(&self) -> (String, String) {
        let link = self.host.link().unwrap();
        let secret = link.split_once("#pair=").unwrap().1;
        let mut response = self
            .agent
            .post(self.api("/auth/pair"))
            .header("Origin", self.host.origin())
            .send_json(json!({"secret": secret, "device_name": "test browser"}))
            .unwrap();
        assert_eq!(response.status(), 200);
        let cookie = response
            .headers()
            .get("set-cookie")
            .unwrap()
            .to_str()
            .unwrap();
        assert!(cookie.contains("HttpOnly; SameSite=Strict"));
        let cookie = cookie.split(';').next().unwrap().to_owned();
        let data: Value = response.body_mut().read_json().unwrap();
        (cookie, data["csrf_token"].as_str().unwrap().to_owned())
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.stop();
    }
}

#[test]
fn browser_scope_and_revocation() {
    let fixture = Fixture::new();
    let (cookie, csrf) = fixture.pair();
    let auth = fixture.api("/auth/session");
    assert_eq!(fixture.agent.get(&auth).call().unwrap().status(), 401);
    assert_eq!(
        fixture
            .agent
            .get(&auth)
            .header("Cookie", &cookie)
            .header("Host", "other.test")
            .call()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        fixture
            .agent
            .get(&auth)
            .header("Cookie", &cookie)
            .header("Tailscale-Funnel-Request", "?1")
            .call()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        fixture
            .agent
            .get(&auth)
            .header("Cookie", &cookie)
            .call()
            .unwrap()
            .status(),
        200
    );
    let logout = fixture.api("/auth/logout");
    assert_eq!(
        fixture
            .agent
            .post(&logout)
            .header("Origin", "https://other.test")
            .header("Cookie", &cookie)
            .header("X-CSRF-Token", &csrf)
            .send_empty()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        fixture
            .agent
            .post(&logout)
            .header("Origin", fixture.host.origin())
            .header("Cookie", &cookie)
            .send_empty()
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        fixture
            .agent
            .post(&logout)
            .header("Origin", fixture.host.origin())
            .header("Cookie", &cookie)
            .header("X-CSRF-Token", &csrf)
            .send_empty()
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        fixture
            .agent
            .get(&auth)
            .header("Cookie", &cookie)
            .call()
            .unwrap()
            .status(),
        401
    );
}

#[test]
fn repeated_http_send_recovers_the_original_receipt() {
    let fixture = Fixture::new();
    let (cookie, csrf) = fixture.pair();
    let expires = (now() + 180).to_string();
    let submit = |text: &str| {
        fixture
            .agent
            .post(fixture.api("/sessions/42/runs"))
            .header("Origin", fixture.host.origin())
            .header("Cookie", &cookie)
            .header("X-CSRF-Token", &csrf)
            .header("X-Conduit-Epoch", fixture.host.epoch())
            .header("Idempotency-Key", "test-original-request-key")
            .header("X-Conduit-Request-Expires", &expires)
            .header("If-Match", "\"version\"")
            .send_json(json!({"text": text}))
            .unwrap()
    };
    assert_eq!(submit("hello").status(), 202);
    assert_eq!(submit("hello").status(), 202);
    assert_eq!(submit("changed").status(), 409);
    let Request::Write {
        device,
        operation,
        command: Write::Send {
            session: 42, text, ..
        },
    } = fixture.requests.try_recv().unwrap()
    else {
        panic!("expected the original desktop command");
    };
    assert_eq!(text, "hello");
    assert!(fixture.requests.try_recv().is_err());
    assert!(fixture.host.can_apply(&device, &operation));
    fixture.host.saving(&operation, json!({"run_id": "run"}));
    let read = || -> Value {
        fixture
            .agent
            .get(fixture.api("/operations/test-original-request-key"))
            .header("Cookie", &cookie)
            .call()
            .unwrap()
            .body_mut()
            .read_json()
            .unwrap()
    };
    assert_eq!(read()["state"], "saving");
    fixture
        .host
        .complete(&operation, Ok(json!({"saved": true})));
    let done = read();
    assert_eq!(done["state"], "succeeded");
    assert_eq!(done["result"]["saved"], true);
}

#[test]
fn run_approval_route_queues_only_the_decoded_action() {
    let fixture = Fixture::new();
    let (cookie, csrf) = fixture.pair();
    let expires = (now() + 180).to_string();
    let path = fixture.api("/sessions/42/runs/run-7/approvals/ticket-3");
    let submit = |key: &str, body: Value| {
        fixture
            .agent
            .post(&path)
            .header("Origin", fixture.host.origin())
            .header("Cookie", &cookie)
            .header("X-CSRF-Token", &csrf)
            .header("X-Conduit-Epoch", fixture.host.epoch())
            .header("Idempotency-Key", key)
            .header("X-Conduit-Request-Expires", &expires)
            .send_json(body)
            .unwrap()
    };
    assert_eq!(
        submit(
            "test-approval-action",
            json!({"kind":"decision","allow":true})
        )
        .status(),
        202
    );
    let Request::Write {
        command:
            Write::Approval {
                session,
                run,
                ticket,
                action: ApprovalAction::Decide { allow },
            },
        ..
    } = fixture.requests.try_recv().unwrap()
    else {
        panic!("expected a run-bound approval decision");
    };
    assert_eq!(
        (session, run.as_str(), ticket.as_str(), allow),
        (42, "run-7", "ticket-3", true)
    );

    assert_eq!(
        submit(
            "test-question-answer",
            json!({"kind":"answer","answers":{"choice":["A"]}})
        )
        .status(),
        202
    );
    let Request::Write {
        command:
            Write::Approval {
                action: ApprovalAction::Answer { answers },
                ..
            },
        ..
    } = fixture.requests.try_recv().unwrap()
    else {
        panic!("expected a structured answer");
    };
    assert_eq!(answers["choice"], vec!["A"]);
    let operation: Value = fixture
        .agent
        .get(fixture.api("/operations/test-question-answer"))
        .header("Cookie", &cookie)
        .call()
        .unwrap()
        .body_mut()
        .read_json()
        .unwrap();
    assert!(!operation.to_string().contains("answers"));
    assert_eq!(
        submit(
            "test-invalid-action",
            json!({"kind":"decision","allow":false,"extra":true})
        )
        .status(),
        400
    );
    assert!(fixture.requests.try_recv().is_err());
}

#[test]
fn local_http_transport_pairs_without_secure_cookie() {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (host, _requests) = Host::launch(listener, &origin, false).unwrap();
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .proxy(None)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(5)))
        .build()
        .into();
    let link = host.link().unwrap();
    let secret = link.split_once("#pair=").unwrap().1;
    let response = agent
        .post(format!("{origin}/api/remote/v1/auth/pair"))
        .header("Origin", &origin)
        .send_json(json!({"secret": secret, "device_name": "lan browser"}))
        .unwrap();
    assert_eq!(response.status(), 200);
    let cookie = response
        .headers()
        .get("set-cookie")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly; SameSite=Strict"));
    assert!(!cookie.contains("Secure"));
    host.stop();
}

#[test]
fn lan_listener_rejects_non_private_addresses() {
    assert!(Host::bind_lan(Ipv4Addr::LOCALHOST, 0).is_err());
    assert!(Host::bind_lan(Ipv4Addr::new(203, 0, 113, 8), 0).is_err());
}
