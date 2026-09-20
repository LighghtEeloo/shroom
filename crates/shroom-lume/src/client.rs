use std::{
    net::SocketAddr,
    path::{Component, PathBuf},
    time::Duration,
};

use reqwest::{Method, StatusCode, Url};
use serde::{Serialize, de::DeserializeOwned};

use crate::{Accepted, CreateVm, Error, Guest, Operation, Result, Vm, VmName};

const RESPONSE_LIMIT: usize = 1024 * 1024;

/// The Lume service and storage are installed/prepared separately.
#[derive(Clone, Debug)]
pub struct Config {
    /// A literal loopback address with a nonzero port, usually 127.0.0.1:7777.
    pub address: SocketAddr,
    /// An absolute directory on the same host, explicitly sent on every request.
    pub storage: PathBuf,
    /// Bounds the entire HTTP exchange, including reading the response body.
    pub request_timeout: Duration,
}

impl Config {
    fn validate(&self) -> Result<()> {
        if !self.address.ip().is_loopback() || self.address.port() == 0 {
            return Err(Error::InvalidConfig(
                "the HTTP service must use a loopback address and nonzero port",
            ));
        }
        if self.request_timeout.is_zero() || self.request_timeout > Duration::from_secs(3600) {
            return Err(Error::InvalidConfig(
                "request timeout must be greater than zero and at most one hour",
            ));
        }
        if !self.storage.is_absolute()
            || self
                .storage
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || self
                .storage
                .to_str()
                .is_none_or(|path| path.contains('%') || path.chars().any(char::is_control))
        {
            // Lume percent-decodes storage queries twice. Reject literal percent
            // characters so an encoded path cannot select a different directory.
            return Err(Error::InvalidConfig(
                "storage must be an absolute UTF-8 path without parent components, percent, or control characters",
            ));
        }
        Ok(())
    }
}

/// Typed HTTP operations. Clones share a connection pool, not a VM ownership lock.
#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base: Url,
    storage: String,
    timeout: Duration,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CreateRequest<'a> {
    name: &'a VmName,
    os: &'static str,
    cpu: u16,
    memory: String,
    disk_size: String,
    display: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    ipsw: Option<&'a str>,
    storage: &'a str,
    network: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CloneRequest<'a> {
    name: &'a VmName,
    new_name: &'a VmName,
    source_location: &'a str,
    dest_location: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunRequest<'a> {
    no_display: bool,
    vnc: &'static str,
    clipboard: bool,
    shared_directories: [(); 0],
    recovery_mode: bool,
    network: &'static str,
    storage: &'a str,
}

#[derive(Serialize)]
struct StorageRequest<'a> {
    storage: &'a str,
}

#[derive(serde::Deserialize)]
struct ApiError {
    message: String,
}

impl Client {
    /// Validate local configuration and construct a client without contacting Lume.
    pub fn new(config: Config) -> Result<Self> {
        config.validate()?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .timeout(config.request_timeout)
            .build()
            .map_err(Error::HttpClient)?;
        Ok(Self {
            http,
            base: Url::parse(&format!("http://{}/lume/", config.address))
                .expect("validated socket address forms an HTTP URL"),
            storage: config.storage.to_str().expect("validated UTF-8").to_owned(),
            timeout: config.request_timeout,
        })
    }

    pub async fn list(&self) -> Result<Vec<Vm>> {
        let request = self.http.get(self.scoped_url(&["vms"]));
        let bytes = self
            .exchange(Operation::List, request, StatusCode::OK)
            .await?;
        Self::decode(Operation::List, &bytes)
    }

    /// Preserve native HTTP errors. In particular, HTTP 400 does not mean "not found".
    pub async fn get(&self, name: &VmName) -> Result<Vm> {
        let request = self.http.get(self.scoped_url(&["vms", name.as_str()]));
        let bytes = self
            .exchange(Operation::Get, request, StatusCode::OK)
            .await?;
        let vm: Vm = Self::decode(Operation::Get, &bytes)?;
        Self::check_name(Operation::Get, name, &vm.name)?;
        Ok(vm)
    }

    /// Start asynchronous creation. Poll `get` to observe progress.
    /// A Linux create allocates an empty VM; it does not install a Linux OS.
    pub async fn create(&self, vm: &CreateVm) -> Result<Accepted> {
        vm.validate()?;
        let (os, ipsw) = match &vm.guest {
            Guest::Linux => ("linux", None),
            Guest::MacOs { restore_image } => ("macos", restore_image.to_str()),
        };
        let body = CreateRequest {
            name: &vm.name,
            os,
            cpu: vm.cpus,
            // Lume's MB/GB spellings represent powers of 1024; it rejects MiB/GiB.
            memory: format!("{}MB", vm.memory_mib),
            disk_size: format!("{}GB", vm.disk_gib),
            display: "1024x768",
            ipsw,
            storage: &self.storage,
            network: "nat",
        };
        let request = self.http.post(self.url(&["vms"])).json(&body);
        self.accepted(Operation::Create, &vm.name, request).await
    }

    /// Clone a stopped prepared VM within this client's storage scope.
    /// Cloning also copies guest credentials; this is not SSH provisioning.
    pub async fn clone_vm(&self, source: &VmName, destination: &VmName) -> Result<()> {
        let request = self
            .http
            .post(self.url(&["vms", "clone"]))
            .json(&CloneRequest {
                name: source,
                new_name: destination,
                source_location: &self.storage,
                dest_location: &self.storage,
            });
        self.exchange(Operation::Clone, request, StatusCode::OK)
            .await?;
        Ok(())
    }

    /// Start headlessly with VNC, clipboard, and directory sharing disabled.
    /// NAT does not provide Shroom's public-only egress policy or a loopback SSH port.
    /// The service owns the accepted run, independently of this client's lifetime.
    pub async fn start(&self, name: &VmName) -> Result<Accepted> {
        let request = self
            .http
            .post(self.url(&["vms", name.as_str(), "run"]))
            .json(&RunRequest {
                no_display: true,
                vnc: "disabled",
                clipboard: false,
                shared_directories: [],
                recovery_mode: false,
                network: "nat",
                storage: &self.storage,
            });
        self.accepted(Operation::Start, name, request).await
    }

    /// Native stop may cut VM power or kill its host process. It is deliberately
    /// named `force_stop` and must not implement Shroom Core's graceful `stop`.
    pub async fn force_stop(&self, name: &VmName) -> Result<()> {
        // Unlike GET/DELETE, upstream reads this endpoint's storage from JSON.
        let request = self
            .http
            .post(self.url(&["vms", name.as_str(), "stop"]))
            .json(&StorageRequest {
                storage: &self.storage,
            });
        self.exchange(Operation::ForceStop, request, StatusCode::OK)
            .await?;
        Ok(())
    }

    /// Native deletion may stop a running VM and permanently removes its disks.
    /// Upstream has no atomic "delete only if stopped" API.
    pub async fn force_delete(&self, name: &VmName) -> Result<()> {
        let request = self
            .http
            .request(Method::DELETE, self.scoped_url(&["vms", name.as_str()]));
        self.exchange(Operation::ForceDelete, request, StatusCode::OK)
            .await?;
        Ok(())
    }

    fn url(&self, segments: &[&str]) -> Url {
        let mut url = self.base.clone();
        url.path_segments_mut()
            .expect("HTTP URL supports path segments")
            .pop_if_empty()
            .extend(segments.iter().copied());
        url
    }

    fn scoped_url(&self, segments: &[&str]) -> Url {
        let mut url = self.url(segments);
        url.query_pairs_mut().append_pair("storage", &self.storage);
        // Foundation URLComponents decodes percent escapes but leaves '+' literal.
        // Query serialization encodes literal '+' as %2B, so only spaces need fixing.
        let query = url
            .query()
            .expect("storage query was appended")
            .replace('+', "%20");
        url.set_query(Some(&query));
        url
    }

    fn check_name(operation: Operation, requested: &VmName, returned: &VmName) -> Result<()> {
        if requested != returned {
            return Err(Error::IdentityMismatch { operation });
        }
        Ok(())
    }

    async fn accepted(
        &self,
        operation: Operation,
        name: &VmName,
        request: reqwest::RequestBuilder,
    ) -> Result<Accepted> {
        let bytes = self
            .exchange(operation, request, StatusCode::ACCEPTED)
            .await?;
        let accepted: Accepted = Self::decode(operation, &bytes)?;
        Self::check_name(operation, name, &accepted.name)?;
        Ok(accepted)
    }

    fn decode<T: DeserializeOwned>(operation: Operation, bytes: &[u8]) -> Result<T> {
        serde_json::from_slice(bytes).map_err(|source| Error::Decode { operation, source })
    }

    async fn exchange(
        &self,
        operation: Operation,
        request: reqwest::RequestBuilder,
        expected: StatusCode,
    ) -> Result<Vec<u8>> {
        tokio::time::timeout(self.timeout, async {
            let transport = |source: reqwest::Error| {
                if source.is_timeout() {
                    Error::Timeout { operation }
                } else {
                    Error::Transport { operation, source }
                }
            };
            let mut response = request.send().await.map_err(transport)?;
            let status = response.status();
            if response
                .content_length()
                .is_some_and(|length| length > RESPONSE_LIMIT as u64)
            {
                return Err(Error::ResponseTooLarge {
                    operation,
                    limit: RESPONSE_LIMIT,
                });
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(transport)? {
                if chunk.len() > RESPONSE_LIMIT - bytes.len() {
                    return Err(Error::ResponseTooLarge {
                        operation,
                        limit: RESPONSE_LIMIT,
                    });
                }
                bytes.extend_from_slice(&chunk);
            }
            if status != expected {
                let message = serde_json::from_slice::<ApiError>(&bytes)
                    .map(|error| error.message)
                    .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).into_owned())
                    .chars()
                    .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
                    .take(1024)
                    .collect();
                return Err(Error::Api {
                    operation,
                    status: status.as_u16(),
                    message,
                });
            }
            Ok(bytes)
        })
        .await
        .map_err(|_| Error::Timeout { operation })?
    }
}
