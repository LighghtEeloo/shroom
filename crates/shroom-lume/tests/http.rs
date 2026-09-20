//! Wire fixtures follow the upstream Swift request, response, and route types at
//! REVIEWED_REVISION. These exercise real HTTP transport, not VM virtualization.

use std::{net::SocketAddr, time::Duration};

use serde_json::{Value, json};
use shroom_lume::{Client, Config, CreateVm, Error, Guest, GuestOs, Operation, VmName, VmState};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinHandle,
    time,
};

struct Request {
    method: String,
    url: reqwest::Url,
    body: Value,
}

impl Request {
    async fn read(socket: &mut TcpStream) -> Self {
        let mut bytes = Vec::new();
        let boundary = loop {
            let byte = socket.read_u8().await.unwrap();
            bytes.push(byte);
            assert!(bytes.len() < 32 * 1024);
            if bytes.ends_with(b"\r\n\r\n") {
                break bytes.len();
            }
        };
        let headers = String::from_utf8(bytes).unwrap();
        let mut lines = headers.lines();
        let mut request_line = lines.next().unwrap().split_whitespace();
        let method = request_line.next().unwrap().to_owned();
        let url = reqwest::Url::parse(&format!("http://localhost{}", request_line.next().unwrap()))
            .unwrap();
        let length: usize = lines
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .map(|(_, value)| value.trim().parse().unwrap())
            .unwrap_or(0);
        assert!(length + boundary < 32 * 1024);
        let mut body = vec![0; length];
        socket.read_exact(&mut body).await.unwrap();
        Self {
            method,
            url,
            body: if body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&body).unwrap()
            },
        }
    }

    fn assert_query_storage(&self) {
        assert_eq!(
            self.url.query(),
            Some("storage=%2Ftmp%2Fshroom%20lune%20%26%20friends")
        );
        assert_eq!(
            self.url.query_pairs().collect::<Vec<_>>(),
            [("storage".into(), "/tmp/shroom lune & friends".into())]
        );
    }
}

struct Response {
    status: u16,
    body: Vec<u8>,
    headers: String,
    delay_body: Duration,
    chunked: bool,
}

impl Response {
    fn with_json(status: u16, body: Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(&body).unwrap(),
            headers: String::new(),
            delay_body: Duration::ZERO,
            chunked: false,
        }
    }

    async fn write(self, socket: &mut TcpStream) {
        let framing = if self.chunked {
            "Transfer-Encoding: chunked\r\n".into()
        } else {
            format!("Content-Length: {}\r\n", self.body.len())
        };
        let head = format!(
            "HTTP/1.1 {} Fixture\r\nContent-Type: application/json\r\nConnection: close\r\n{}{framing}\r\n",
            self.status, self.headers
        );
        socket.write_all(head.as_bytes()).await.unwrap();
        time::sleep(self.delay_body).await;
        // Oversize/deadline tests intentionally close before the body finishes.
        if self.chunked {
            for chunk in self.body.chunks(16 * 1024) {
                if socket
                    .write_all(format!("{:x}\r\n", chunk.len()).as_bytes())
                    .await
                    .is_err()
                    || socket.write_all(chunk).await.is_err()
                    || socket.write_all(b"\r\n").await.is_err()
                {
                    return;
                }
            }
            let _ = socket.write_all(b"0\r\n\r\n").await;
        } else {
            let _ = socket.write_all(&self.body).await;
        }
    }
}

struct Server {
    client: Client,
    task: JoinHandle<Vec<Request>>,
}

impl Server {
    async fn new(responses: Vec<Response>, request_timeout: Duration) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::new(Self::config(
            listener.local_addr().unwrap(),
            request_timeout,
        ))
        .unwrap();
        let task = tokio::spawn(async move {
            let mut requests = Vec::new();
            for response in responses {
                let (mut socket, _) = time::timeout(Duration::from_secs(3), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
                requests.push(Request::read(&mut socket).await);
                response.write(&mut socket).await;
            }
            // A failed mutation must not be retried after the first response.
            assert!(
                time::timeout(Duration::from_millis(40), listener.accept())
                    .await
                    .is_err()
            );
            requests
        });
        Self { client, task }
    }

    fn config(address: SocketAddr, request_timeout: Duration) -> Config {
        Config {
            address,
            storage: "/tmp/shroom lune & friends".into(),
            request_timeout,
        }
    }

    fn vm(name: &str, os: &str, state: &str) -> Value {
        json!({
            "name": name, "os": os, "cpuCount": 2, "memorySize": 4294967296_u64,
            "diskSize": { "allocated": 100, "total": 21474836480_u64 },
            "display": "1024x768", "status": state, "vncUrl": null,
            "ipAddress": "192.168.64.2", "sshAvailable": false,
            "locationName": "/tmp/shroom lune & friends", "sharedDirectories": [],
            "provisioningOperation": null, "downloadProgress": null
        })
    }
}

#[tokio::test]
async fn linux_and_macos_lifecycle_use_native_http_shapes_and_explicit_storage() {
    let linux: VmName = "linux".parse().unwrap();
    let mac: VmName = "mac".parse().unwrap();
    let server = Server::new(vec![
        Response::with_json(200, json!([Server::vm("linux", "linux", "stopped"), Server::vm("mac", "macOS", "running")])),
        Response::with_json(200, Server::vm("linux", "linux", "stopped")),
        Response::with_json(202, json!({"name": "linux", "status": "provisioning"})),
        Response::with_json(202, json!({"name": "mac", "status": "provisioning"})),
        Response::with_json(200, json!({"message": "VM cloned successfully", "source": "linux", "destination": "work"})),
        Response::with_json(202, json!({"message": "VM start initiated", "name": "linux", "status": "pending"})),
        Response::with_json(200, json!({"message": "VM stopped successfully"})),
        Response { body: Vec::new(), ..Response::with_json(200, Value::Null) },
    ], Duration::from_secs(2)).await;
    let vms = server.client.list().await.unwrap();
    assert_eq!(vms[0].os, Some(GuestOs::Linux));
    assert_eq!(vms[1].os, Some(GuestOs::MacOs));
    assert_eq!(vms[1].state, VmState::Running);
    assert_eq!(vms[1].ssh_available, Some(false));
    assert_eq!(
        server.client.get(&linux).await.unwrap().state,
        VmState::Stopped
    );
    for (name, guest) in [
        (linux.clone(), Guest::Linux),
        (
            mac.clone(),
            Guest::MacOs {
                restore_image: "/tmp/macOS.ipsw".into(),
            },
        ),
    ] {
        let accepted = server
            .client
            .create(&CreateVm {
                name,
                guest,
                cpus: 2,
                memory_mib: 4096,
                disk_gib: 20,
            })
            .await
            .unwrap();
        assert_eq!(accepted.state, VmState::Provisioning);
    }
    server
        .client
        .clone_vm(&linux, &"work".parse().unwrap())
        .await
        .unwrap();
    assert_eq!(
        server.client.start(&linux).await.unwrap().state,
        VmState::Pending
    );
    server.client.force_stop(&linux).await.unwrap();
    server.client.force_delete(&linux).await.unwrap();
    let requests = server.task.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .map(|r| (r.method.as_str(), r.url.path()))
            .collect::<Vec<_>>(),
        [
            ("GET", "/lume/vms"),
            ("GET", "/lume/vms/linux"),
            ("POST", "/lume/vms"),
            ("POST", "/lume/vms"),
            ("POST", "/lume/vms/clone"),
            ("POST", "/lume/vms/linux/run"),
            ("POST", "/lume/vms/linux/stop"),
            ("DELETE", "/lume/vms/linux")
        ]
    );
    for index in [0, 1, 7] {
        requests[index].assert_query_storage();
    }
    for index in [2, 3, 5, 6] {
        assert_eq!(
            requests[index].body["storage"],
            "/tmp/shroom lune & friends"
        );
    }
    assert_eq!(requests[2].body["os"], "linux");
    assert!(requests[2].body.get("ipsw").is_none());
    assert_eq!(requests[3].body["os"], "macos");
    assert_eq!(requests[3].body["ipsw"], "/tmp/macOS.ipsw");
    assert_eq!(requests[3].body["memory"], "4096MB");
    assert_eq!(requests[3].body["diskSize"], "20GB");
    assert_eq!(
        requests[4].body["sourceLocation"],
        "/tmp/shroom lune & friends"
    );
    assert_eq!(
        requests[4].body["destLocation"],
        "/tmp/shroom lune & friends"
    );
    assert_eq!(
        requests[5].body,
        json!({"noDisplay":true,"vnc":"disabled","clipboard":false,"sharedDirectories":[],"recoveryMode":false,"network":"nat","storage":"/tmp/shroom lune & friends"})
    );
    assert!(
        requests[6].url.query().is_none(),
        "stop storage belongs in JSON, not its query"
    );
}

#[tokio::test]
async fn invalid_inputs_are_rejected_before_any_http_request() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let config = Server::config(listener.local_addr().unwrap(), Duration::from_secs(1));
    assert!(Client::new(config.clone()).is_ok());
    for address in [
        "0.0.0.0:7777",
        "192.168.1.1:7777",
        "127.0.0.1:0",
        "[::]:7777",
    ] {
        assert!(matches!(
            Client::new(Config {
                address: address.parse().unwrap(),
                ..config.clone()
            }),
            Err(Error::InvalidConfig(_))
        ));
    }
    for storage in [
        "relative",
        "/tmp/../elsewhere",
        "/tmp/literal%2F",
        "/tmp/new\nline",
    ] {
        assert!(matches!(
            Client::new(Config {
                storage: storage.into(),
                ..config.clone()
            }),
            Err(Error::InvalidConfig(_))
        ));
    }
    for timeout in [Duration::ZERO, Duration::from_secs(3601)] {
        assert!(matches!(
            Client::new(Config {
                request_timeout: timeout,
                ..config.clone()
            }),
            Err(Error::InvalidConfig(_))
        ));
    }
    for name in ["a", "0", "project-1", &"a".repeat(48)] {
        assert!(name.parse::<VmName>().is_ok());
    }
    for name in [
        "",
        "-a",
        "../vm",
        "base:latest",
        "vm?x=y",
        "vm/name",
        "A",
        "a\n",
        &"a".repeat(49),
    ] {
        assert!(matches!(name.parse::<VmName>(), Err(Error::InvalidName)));
    }
    let client = Client::new(config).unwrap();
    let good = CreateVm {
        name: "valid".parse().unwrap(),
        guest: Guest::Linux,
        cpus: 2,
        memory_mib: 4096,
        disk_gib: 20,
    };
    for bad in [
        CreateVm {
            cpus: 0,
            ..good.clone()
        },
        CreateVm {
            memory_mib: 0,
            ..good.clone()
        },
        CreateVm {
            disk_gib: 0,
            ..good.clone()
        },
        CreateVm {
            guest: Guest::MacOs {
                restore_image: "relative.ipsw".into(),
            },
            ..good.clone()
        },
    ] {
        assert!(matches!(
            client.create(&bad).await,
            Err(Error::InvalidCreate(_))
        ));
    }
    assert!(
        time::timeout(Duration::from_millis(40), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn native_errors_unknown_states_and_identity_mismatches_are_preserved() {
    let server = Server::new(
        vec![
            Response::with_json(
                200,
                json!({"name":"vm","status":"pulling","downloadProgress":42.5}),
            ),
            Response::with_json(200, Server::vm("vm", "linux", "resizing")),
            Response::with_json(200, Server::vm("vm", "linux", "provisioning (stale)")),
            Response::with_json(400, json!({"message":"IPSW file not found"})),
            Response::with_json(200, Server::vm("other", "linux", "stopped")),
            Response::with_json(200, json!({"name":"vm"})),
            Response::with_json(
                200,
                json!({"name":"vm","status":"running","ipAddress":"invalid"}),
            ),
            Response::with_json(202, json!({"name":"other","status":"pending"})),
        ],
        Duration::from_secs(2),
    )
    .await;
    let name = "vm".parse().unwrap();
    let pulling = server.client.get(&name).await.unwrap();
    assert_eq!(pulling.state, VmState::Pulling);
    assert_eq!(pulling.os, None);
    assert_eq!(pulling.download_progress, Some(42.5));
    assert_eq!(
        server.client.get(&name).await.unwrap().state,
        VmState::Unknown("resizing".into())
    );
    assert_eq!(
        server.client.get(&name).await.unwrap().state,
        VmState::StaleProvisioning
    );
    assert!(
        matches!(server.client.get(&name).await, Err(Error::Api { operation: Operation::Get, status: 400, message }) if message == "IPSW file not found")
    );
    assert!(matches!(
        server.client.get(&name).await,
        Err(Error::IdentityMismatch {
            operation: Operation::Get
        })
    ));
    for _ in 0..2 {
        assert!(matches!(
            server.client.get(&name).await,
            Err(Error::Decode {
                operation: Operation::Get,
                ..
            })
        ));
    }
    assert!(matches!(
        server.client.start(&name).await,
        Err(Error::IdentityMismatch {
            operation: Operation::Start
        })
    ));
    server.task.await.unwrap();
}

#[tokio::test]
async fn redirects_and_failed_mutations_are_never_replayed() {
    let redirected = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server = Server::new(
        vec![
            Response {
                headers: format!(
                    "Location: http://{}/stolen\r\n",
                    redirected.local_addr().unwrap()
                ),
                ..Response::with_json(307, json!({"message":"redirect"}))
            },
            Response::with_json(503, json!({"message":"unavailable"})),
        ],
        Duration::from_secs(1),
    )
    .await;
    let name = "vm".parse().unwrap();
    assert!(matches!(
        server.client.start(&name).await,
        Err(Error::Api { status: 307, .. })
    ));
    assert!(matches!(
        server.client.force_delete(&name).await,
        Err(Error::Api { status: 503, .. })
    ));
    assert_eq!(server.task.await.unwrap().len(), 2);
    assert!(
        time::timeout(Duration::from_millis(40), redirected.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn response_bodies_are_bounded_with_and_without_content_length() {
    let server = Server::new(
        [false, true]
            .into_iter()
            .map(|chunked| Response {
                body: vec![b' '; 1024 * 1024 + 1],
                chunked,
                ..Response::with_json(200, Value::Null)
            })
            .collect(),
        Duration::from_secs(2),
    )
    .await;
    for _ in 0..2 {
        assert!(matches!(
            server.client.list().await,
            Err(Error::ResponseTooLarge {
                operation: Operation::List,
                ..
            })
        ));
    }
    server.task.await.unwrap();
}

#[tokio::test]
async fn deadline_covers_a_stalled_response_body() {
    let server = Server::new(
        vec![Response {
            delay_body: Duration::from_millis(150),
            ..Response::with_json(200, json!([]))
        }],
        Duration::from_millis(40),
    )
    .await;
    assert!(matches!(
        server.client.list().await,
        Err(Error::Timeout {
            operation: Operation::List
        })
    ));
    server.task.await.unwrap();
}
