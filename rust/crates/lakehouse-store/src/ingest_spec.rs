//! Per-adapter `dial` validation for registry-driven ingestion
//! (`docs/adr/0013-registry-driven-ingestion.md`).
//!
//! `connector.dial` (`0033_connector_ingest_spec.sql`) is free-form
//! `JSONB` at the column level, but never free-form at the application
//! level: [`Dial::parse`] parses it into exactly one of eight
//! `#[serde(deny_unknown_fields, rename_all = "camelCase")]` structs,
//! dispatched on the connector's own `adapter` column value. Every one of
//! the eight structs rejects an unknown field, which is the property that
//! stops a credential (a `password`, an `apiSecret`) being smuggled into
//! `dial` — a column the allowlisted `secretRef` resolver
//! (`lakehouse_core::secret`, ADR 0002) never inspects.
//!
//! # Never `#[serde(untagged)]`
//!
//! [`Dial::parse`] dispatches on the stored `adapter` column value and
//! then parses against exactly one struct. An untagged enum would instead
//! try every variant in turn: a malformed `dial` for a known adapter could
//! silently match a structurally-similar but wrong shape instead of
//! failing with that adapter's own named field-path error. See ADR 0013's
//! "Never `#[serde(untagged)]`" consequence for the full argument.

use serde::Deserialize;

/// A closed set of SQL/CDC drivers `dial` may name — never a free-form
/// string, so a typo (`"postgress"`) fails at parse time rather than at
/// the point some later code tries to dispatch on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SqlDriver {
    /// `MySQL`/`MariaDB`.
    Mysql,
    /// `PostgreSQL`.
    Postgres,
    /// Microsoft SQL Server, dialed via ODBC by the Dagster `sql` adapter
    /// this driver's `dial` values are validated for.
    Mssql,
    /// Oracle, thin-mode `oracledb` (`dagster/dispar_orchestrate/adapters/
    /// oracle.py`). Reuses the existing `sql` `dial`/`Dial::parse` shape
    /// — Oracle is the fourth `SqlDriver` variant, not a fourth string
    /// `adapter` value, so `connector_type.adapter = 'sql'` covers it.
    Oracle,
}

/// The connection shape shared by a batch `sql` adapter's `dial`.
///
/// `user` is a literal, non-`secretRef`-shaped string — never
/// `"env:CONNECTOR_<ID>_USER"` or any other indirection scheme. This is a
/// deliberate correction against an earlier draft that put `user` behind
/// the same `secretRef`-style indirection as a password: a username is
/// not a credential, and the seeded `conn-pg-lakehouse` row already
/// encodes its username directly in `host`
/// (`lakehouse@postgres:5432/lakehouse`,
/// `0022_prune_connector_seed.sql`) rather than through any resolver — see
/// `docs/adr/0013-registry-driven-ingestion.md`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SqlDial {
    /// The SQL driver to dial.
    pub driver: SqlDriver,
    /// The hostname or IP literal to connect to, validated by
    /// [`validate_hostname`] before this `dial` is accepted.
    pub host: String,
    /// The TCP port to connect to.
    pub port: u16,
    /// The database (or schema, for engines without a separate concept)
    /// name.
    pub database: String,
    /// A literal username, never a `secretRef`-shaped indirection — see
    /// this module's own doc comment and [`SqlDial`]'s doc comment.
    pub user: String,
    /// An optional TLS mode string, passed through verbatim to the
    /// generated ingestion job.
    pub ssl_mode: Option<String>,
    /// An optional, operator-supplied Distinguished Name the Oracle
    /// adapter's thin-mode `oracledb` driver uses for TLS certificate
    /// verification. Required whenever `ssl_mode` implies TLS for an
    /// Oracle connection — [`Dial::parse`] enforces this for the
    /// `sql`/Oracle case; `mysql`/`postgresql`/`mssql` ignore the field.
    /// Never synthesized from `host` (a bare `CN=<hostname>` would not
    /// match a real certificate's full DN).
    #[serde(default)]
    pub ssl_server_cert_dn: Option<String>,
}

/// The connection shape for a change-data-capture `cdc` adapter's `dial`.
///
/// `slot_name`/`publication_name` are **required**, not `Option`: a `cdc`
/// connector's replication slot/publication come from `dial` with no
/// fallback, so a `cdc` dial missing either is a construction-time (400)
/// error, not a later runtime surprise — see
/// `docs/adr/0013-registry-driven-ingestion.md`'s "the same connection
/// fields as `sql`, plus `slotName` and `publicationName` as **required**"
/// decision.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CdcDial {
    /// The SQL driver to dial.
    pub driver: SqlDriver,
    /// The hostname or IP literal to connect to, validated by
    /// [`validate_hostname`] before this `dial` is accepted.
    pub host: String,
    /// The TCP port to connect to.
    pub port: u16,
    /// The database (or schema, for engines without a separate concept)
    /// name.
    pub database: String,
    /// A literal username, never a `secretRef`-shaped indirection — see
    /// [`SqlDial::user`].
    pub user: String,
    /// The replication slot name, required with no fallback.
    pub slot_name: String,
    /// The publication name, required with no fallback.
    pub publication_name: String,
    /// An optional numeric server id, used by drivers (e.g. `MySQL`
    /// binlog replication) that need one.
    pub server_id: Option<u32>,
}

/// A closed set of object-storage protocols a `files` adapter's `dial`
/// may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesProtocol {
    /// S3-compatible object storage (including `RustFS`).
    S3,
    /// SFTP.
    Sftp,
}

/// A closed set of file formats a `files` adapter's `dial` may name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesFormat {
    /// Comma-separated values.
    Csv,
    /// Newline-delimited JSON.
    Json,
    /// Apache Parquet.
    Parquet,
}

/// The connection shape for a `files` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FilesDial {
    /// The object-storage protocol to dial.
    pub protocol: FilesProtocol,
    /// An optional endpoint override (e.g. a `RustFS`/`MinIO` endpoint
    /// URL rather than public AWS S3).
    pub endpoint: Option<String>,
    /// The bucket (or SFTP root) name.
    pub bucket: String,
    /// An optional key/path prefix to scope ingestion to.
    pub prefix: Option<String>,
    /// The file format to parse.
    pub format: FilesFormat,
    /// An optional region string, passed through verbatim.
    pub region: Option<String>,
}

/// One `rest` adapter authentication shape, internally tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum RestAuth {
    /// A named header carries the API key. The key's actual value is
    /// still only ever named by the connector's `secretRef`, never
    /// present in this struct.
    #[serde(rename = "api_key")]
    ApiKey {
        /// The header name the API key is sent under.
        header: String,
    },
    /// A bearer token carried in the `Authorization` header.
    Bearer,
    /// `OAuth2` client-credentials grant.
    #[serde(rename = "oauth2_client_credentials")]
    Oauth2ClientCredentials {
        /// The token endpoint URL to exchange credentials at.
        token_url: String,
    },
    /// HTTP Basic authentication.
    Basic,
}

impl RestAuth {
    /// The `type` tag this variant serializes under — the same string
    /// `secret_map.secret_field_names`/[`secret_field_names`] key on for a
    /// `rest` adapter's `auth_type` (WS3 plan review Z6).
    #[must_use]
    pub fn type_tag(&self) -> &'static str {
        match self {
            RestAuth::ApiKey { .. } => "api_key",
            RestAuth::Bearer => "bearer",
            RestAuth::Oauth2ClientCredentials { .. } => "oauth2_client_credentials",
            RestAuth::Basic => "basic",
        }
    }
}

/// A `rest` adapter's pagination shape, internally tagged on `type`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum RestPagination {
    /// No pagination — a single request returns the full result set.
    None,
    /// Page-number-based pagination.
    Page {
        /// The query parameter name carrying the page number.
        param: String,
    },
    /// Cursor-based pagination.
    Cursor {
        /// The JSON path in a response body naming the next cursor.
        cursor_field: String,
    },
}

/// One endpoint a `rest` adapter's `dial` enumerates.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RestEndpoint {
    /// The path, relative to `RestDial::base_url`.
    pub path: String,
    /// The JSON path in a response body naming the record array, when the
    /// endpoint does not return a bare array at its root.
    pub records_path: Option<String>,
}

/// The connection shape for a `rest` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RestDial {
    /// The base URL every [`RestEndpoint::path`] is relative to.
    pub base_url: String,
    /// The authentication shape to use.
    pub auth: RestAuth,
    /// The pagination shape to use.
    pub pagination: RestPagination,
    /// The endpoints to poll.
    pub endpoints: Vec<RestEndpoint>,
}

/// The connection shape for a `sheets` adapter's `dial`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SheetsDial {
    /// The Google Sheets spreadsheet id.
    pub spreadsheet_id: String,
    /// The A1-notation ranges to read.
    pub ranges: Vec<String>,
}

/// The connection shape for a `mongodb` adapter's `dial`.
///
/// Refuses `mongodb+srv` discovery and replica-set discovery outright
/// (`directConnection` MUST be `true`): both let the driver reach hosts
/// this connector's SSRF check never validated. `MongoDial` has no
/// `srvUri` field by construction —
/// `deny_unknown_fields` enforces this, so a JSON shape carrying an
/// `srvUri` is rejected at deserialize time as an unknown field. The
/// `hosts` list is always explicit, one seed per entry, never discovered
/// at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MongoDial {
    /// The explicit seed hosts, `host:port`-shaped, that the driver
    /// dials directly (no `mongodb+srv`, no replica-set discovery). Every
    /// entry is checked by [`validate_hostname`] before this `dial` is
    /// accepted; the bare-host portion of each is what is checked.
    pub hosts: Vec<String>,
    /// The database name; pymongo also takes this as the auth source.
    pub database: String,
    /// The literal username — a bare username is not credential-shaped
    /// (same reasoning as [`SqlDial::user`]); the password is the
    /// connector's `secretRef` value, never named here.
    pub username: String,
    /// MUST be `true`. Replica-set discovery is refused outright (not
    /// partially checked): it would let the driver reach hosts this
    /// connector's SSRF check never validated.
    #[serde(rename = "directConnection")]
    pub direct_connection: bool,
}

/// The `mongodb` adapter's authentication shape. Carries no fields today
/// because `MongoDial.username` is the literal username and the password
/// is the connector's `secretRef`. Modeled as an internally-tagged enum
/// (rather than `bool`) so future `MongoDB` auth mechanisms (`SCRAM-SHA-256`
/// etc.) have an obvious extension point.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum MongoAuth {
    /// No auth-specific fields today; carries the username via
    /// [`MongoDial::username`] and the password via `secretRef`.
    #[serde(rename = "password")]
    Password,
}

/// The connection shape for a `kafka` adapter's `dial`.
///
/// `bootstrap_servers` are the brokers the operator initially dials; the
/// cluster's metadata response then names the real "advertised listener"
/// addresses the driver will actually connect to per partition. That
/// post-bootstrap check is `ssrf_guard_kafka.check_all_advertised_brokers`,
/// not this struct (the broker list is server-supplied, not
/// operator-supplied). This struct validates the OPERATOR-supplied
/// `bootstrap_servers` shape only.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct KafkaDial {
    /// The bootstrap brokers the operator initially dials, `host:port`-
    /// shaped. Each entry's bare-host portion is checked by
    /// [`validate_hostname`].
    pub bootstrap_servers: Vec<String>,
    /// The topic to consume from.
    pub topic: String,
    /// The authentication shape — `kafka-python` accepts the structured
    /// `sasl_plain_username` / `sasl_plain_password` keyword args; only
    /// the username is named here, the password is the connector's
    /// `secretRef`.
    pub auth: KafkaAuth,
    /// The consumer group id used to commit per-partition offsets. The
    /// Kafka adapter commits to BOTH Kafka's own consumer-group offset
    /// and `bronze_meta.ingest_offset` (Dagster side).
    pub group_id: String,
    /// The wall-clock cap on a single micro-batch read.
    pub micro_batch_seconds: u32,
}

/// The `kafka` adapter's authentication shape. Mirrors
/// [`RestAuth`]'s `tag = "type"` shape so a future `sasl_scram_sha256`
/// variant lands the same way.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum KafkaAuth {
    /// `SASL/PLAIN` over a TLS connection: the username is named here,
    /// the password is the connector's `secretRef`.
    #[serde(rename = "sasl_plain")]
    SaslPlain {
        /// The SASL/PLAIN username.
        username: String,
    },
    /// No SASL — a broker whose `security.protocol` is `PLAINTEXT`. The
    /// connector has no `secretRef` for the password in this case (the
    /// `secret_field_names` mapping for `("kafka", "none")` is empty);
    /// a deployment must keep `secretRef` unset or use a non-credential
    /// reference.
    #[serde(rename = "none")]
    None,
}

impl KafkaAuth {
    /// The `type` tag this variant serializes under — the same string
    /// [`RestAuth::type_tag`] returns for `rest`; reused as the
    /// `auth_type` key the `(adapter, auth type) -> secret fields`
    /// mapping ([`secret_field_names`]) keys on.
    #[must_use]
    pub fn type_tag(&self) -> &'static str {
        match self {
            KafkaAuth::SaslPlain { .. } => "sasl_plain",
            KafkaAuth::None => "none",
        }
    }
}

/// The connection shape for an `sftp` adapter's `dial`.
///
/// `sftp` is its own `adapter` value, not a `FilesProtocol::Sftp`
/// protocol — `dagster/dispar_orchestrate/adapters/files.py` is
/// s3fs/S3-only, and a `files`/`sftp` round-trip would never have a
/// working sink (`adapters/sftp.py` is its own module). `host_key_fingerprint`
/// is **required** with no fallback — `paramiko.AutoAddPolicy` is NEVER
/// acceptable, since it would accept any host key on first connect and
/// defeat host verification entirely; `deny_unknown_fields` plus the
/// missing-field error handle the rejection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SftpDial {
    /// The hostname to connect to, validated by [`validate_hostname`]
    /// before this `dial` is accepted.
    pub host: String,
    /// The TCP port to connect to (typically `22`).
    pub port: u16,
    /// A literal username; may be empty for an anonymous-style flow but
    /// is REQUIRED by the auth path either way (the empty default is for
    /// compatibility with SFTP servers that accept an unauthenticated
    /// listing, never for a real transfer).
    pub user: String,
    /// The pinned SSH host-key fingerprint the operator expects to see
    /// when `paramiko.SSHClient.connect` runs (never `AutoAddPolicy`).
    /// Required, no fallback; a missing field fails to deserialize.
    pub host_key_fingerprint: String,
    /// The remote path the adapter reads from; the `source_objects` rows
    /// are file names within this directory.
    pub path: String,
    /// The file format to parse (currently `csv` only on the Dagster
    /// side).
    pub file_format: String,
    /// The authentication shape — `password` or `public_key`. The
    /// credential itself is the connector's `secretRef`.
    pub auth: SftpAuth,
}

/// The `sftp` adapter's authentication shape.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum SftpAuth {
    /// Username + password (carried as the connector's `secretRef`,
    /// resolved to a `password` field name by [`secret_field_names`]).
    #[serde(rename = "password")]
    Password,
    /// Public-key authentication (the private key PEM is the
    /// connector's `secretRef`, resolved to a `privateKey` field name).
    #[serde(rename = "public_key")]
    PublicKey,
}

impl SftpAuth {
    /// The `type` tag this variant serializes under.
    #[must_use]
    pub fn type_tag(&self) -> &'static str {
        match self {
            SftpAuth::Password => "password",
            SftpAuth::PublicKey => "public_key",
        }
    }
}

/// A parsed `dial`, one variant per adapter.
///
/// Never `#[serde(untagged)]` — see this module's doc comment. Constructed
/// only via [`Dial::parse`], which dispatches on the connector's own
/// `adapter` column value rather than deriving the shape from the JSON
/// itself.
#[derive(Debug, Clone, PartialEq)]
pub enum Dial {
    /// A batch `sql` adapter's dial.
    Sql(SqlDial),
    /// A change-data-capture `cdc` adapter's dial.
    Cdc(CdcDial),
    /// A `mongodb` adapter's dial.
    Mongo(MongoDial),
    /// A `kafka` adapter's dial.
    Kafka(KafkaDial),
    /// An `sftp` adapter's dial.
    Sftp(SftpDial),
    /// A `files` adapter's dial.
    Files(FilesDial),
    /// A `rest` adapter's dial.
    Rest(RestDial),
    /// A `sheets` adapter's dial.
    Sheets(SheetsDial),
}

impl Dial {
    /// Parse `raw` into the [`Dial`] shape named by `adapter`.
    ///
    /// Dispatches on `adapter` — the connector's own stored column value —
    /// and parses `raw` against exactly one struct, never trying every
    /// shape in turn. `host` (or its multi-entry equivalent
    /// `hosts`/`bootstrap_servers`) is additionally checked by
    /// [`validate_hostname`] for `sql`/`cdc`/`mongodb`/`kafka`/`sftp`
    /// adapters, since the Dagster adapter later interpolates the host
    /// into a delimiter-separated text format.
    ///
    /// # Errors
    ///
    /// Returns [`IngestSpecError::UnknownAdapter`] if `adapter` is not one
    /// of `sql`/`cdc`/`mongodb`/`kafka`/`sftp`/`files`/`rest`/`sheets`,
    /// or [`IngestSpecError::InvalidDial`] if `raw` does not match the
    /// named adapter's shape (an unknown field, a missing required field,
    /// a hostname [`validate_hostname`] rejects, an Oracle TLS
    /// configuration without an `ssl_server_cert_dn`, or a control
    /// character in `ssl_server_cert_dn`).
    pub fn parse(adapter: &str, raw: &serde_json::Value) -> Result<Dial, IngestSpecError> {
        match adapter {
            "sql" => {
                let dial: SqlDial = parse_dial(adapter, raw)?;
                check_hostname(adapter, "host", &dial.host)?;
                validate_sql_dial_post_parse(adapter, &dial)?;
                Ok(Dial::Sql(dial))
            }
            "cdc" => {
                let dial: CdcDial = parse_dial(adapter, raw)?;
                check_hostname(adapter, "host", &dial.host)?;
                Ok(Dial::Cdc(dial))
            }
            "mongodb" => {
                let dial: MongoDial = parse_dial(adapter, raw)?;
                for host_port in &dial.hosts {
                    let (host, _port) = split_host_port(adapter, "hosts", host_port)?;
                    check_hostname(adapter, "hosts", host)?;
                }
                Ok(Dial::Mongo(dial))
            }
            "kafka" => {
                let dial: KafkaDial = parse_dial(adapter, raw)?;
                for server in &dial.bootstrap_servers {
                    let (host, _port) = split_host_port(adapter, "bootstrapServers", server)?;
                    check_hostname(adapter, "bootstrapServers", host)?;
                }
                Ok(Dial::Kafka(dial))
            }
            "sftp" => {
                let dial: SftpDial = parse_dial(adapter, raw)?;
                check_hostname(adapter, "host", &dial.host)?;
                Ok(Dial::Sftp(dial))
            }
            "files" => Ok(Dial::Files(parse_dial(adapter, raw)?)),
            "rest" => Ok(Dial::Rest(parse_dial(adapter, raw)?)),
            "sheets" => Ok(Dial::Sheets(parse_dial(adapter, raw)?)),
            other => Err(IngestSpecError::UnknownAdapter {
                adapter: other.to_owned(),
            }),
        }
    }

    /// A typed accessor for the `cdc` variant, so a caller that only cares
    /// about the `cdc` shape (e.g. slot/publication deprovisioning) never
    /// needs its own `match` on the variant name.
    #[must_use]
    pub fn as_cdc(&self) -> Option<&CdcDial> {
        match self {
            Dial::Cdc(dial) => Some(dial),
            _ => None,
        }
    }

    /// The `rest` adapter's `dial.auth.type` tag, or `None` for any other
    /// adapter — the `auth_type` [`secret_field_names`] keys on for
    /// `rest` (WS3 plan review Z6). Used by
    /// [`crate::connectors::set_ingest_spec`] to look up how many secret
    /// refs this spec needs.
    #[must_use]
    pub fn rest_auth_type(&self) -> Option<&'static str> {
        match self {
            Dial::Rest(rest) => Some(rest.auth.type_tag()),
            _ => None,
        }
    }

    /// The `kafka` adapter's `dial.auth.type` tag, or `None` for any
    /// other adapter — the `auth_type` [`secret_field_names`] keys on
    /// for `kafka`. Same role as [`Self::rest_auth_type`] but for the
    /// Kafka dialect (the `(adapter, auth type) -> secret fields` mapping
    /// is the ONE mapping, but its lookup key for `kafka` is the auth
    /// type, not `None`).
    #[must_use]
    pub fn kafka_auth_type(&self) -> Option<&'static str> {
        match self {
            Dial::Kafka(kafka) => Some(kafka.auth.type_tag()),
            _ => None,
        }
    }

    /// The `sftp` adapter's `dial.auth.type` tag, or `None` for any
    /// other adapter — the `auth_type` [`secret_field_names`] keys on
    /// for `sftp`. Same role as [`Self::kafka_auth_type`].
    #[must_use]
    pub fn sftp_auth_type(&self) -> Option<&'static str> {
        match self {
            Dial::Sftp(sftp) => Some(sftp.auth.type_tag()),
            _ => None,
        }
    }

    /// The `auth_type` key [`secret_field_names`] needs for THIS dial's
    /// adapter, whichever of [`Self::rest_auth_type`],
    /// [`Self::kafka_auth_type`] or [`Self::sftp_auth_type`] applies (at
    /// most one ever returns `Some`, since each matches a disjoint set of
    /// `Dial` variants) — the single call [`crate::connectors::set_ingest_spec`]
    /// makes instead of trying each accessor itself.
    #[must_use]
    pub fn secret_map_auth_type(&self) -> Option<&'static str> {
        self.rest_auth_type()
            .or_else(|| self.kafka_auth_type())
            .or_else(|| self.sftp_auth_type())
    }

    /// Where this dial sends the connector's credential, as a comparable
    /// value (`SEC-14`, decision D1). Two dials with equal identities send
    /// a stored credential to the same place; unequal identities mean the
    /// connector is being re-pointed, and the credential must be supplied
    /// again (see [`is_repoint`]).
    ///
    /// | Adapter | Compared |
    /// | --- | --- |
    /// | `sql`, `cdc` | `driver`, `host`, `port`, `database` |
    /// | `files` | `protocol`, `endpoint`, `bucket` |
    /// | `rest` | scheme, host and port of `baseUrl` |
    /// | `mongodb` | `hosts` (as a set), `database` |
    /// | `kafka` | `bootstrapServers` (as a set) |
    /// | `sftp` | `host`, `port`, `hostKeyFingerprint` |
    /// | `sheets` | nothing (fixed Google hosts) |
    ///
    /// The adapter name is part of the identity, so `sql` to `cdc` on the
    /// same host is a re-point. A change of `user`, `sourceObjects`, the
    /// schedule, pagination, `sslMode` or any other field is not.
    ///
    /// # Normalisation
    ///
    /// - Host names compare case-insensitively.
    /// - Ports: the `sql`, `cdc`, `sftp`, `mongodb` and `kafka` shapes have
    ///   no absent port ([`Dial::parse`] refuses a dial without one), so
    ///   there is no default to apply and `5432` versus `5433` differ. In
    ///   a URL (`rest` `baseUrl`, `files` `endpoint`) an absent port means
    ///   the scheme's default, so `https://h` and `https://h:443` are the
    ///   same target and `https://h:8443` is another.
    /// - A URL's userinfo (`user@`) is not part of the host: only what
    ///   follows the last `@` is compared, so `https://good@evil/` is
    ///   `evil`.
    /// - A `files` dial with no `endpoint` (the provider's own default)
    ///   differs from one that names an endpoint.
    #[must_use]
    pub fn target_identity(&self) -> TargetIdentity {
        match self {
            Dial::Sql(sql) => TargetIdentity::new(
                "sql",
                vec![
                    ("driver", driver_key(sql.driver).to_owned()),
                    ("host", host_key(&sql.host)),
                    ("port", sql.port.to_string()),
                    ("database", sql.database.clone()),
                ],
            ),
            Dial::Cdc(cdc) => TargetIdentity::new(
                "cdc",
                vec![
                    ("driver", driver_key(cdc.driver).to_owned()),
                    ("host", host_key(&cdc.host)),
                    ("port", cdc.port.to_string()),
                    ("database", cdc.database.clone()),
                ],
            ),
            Dial::Files(files) => TargetIdentity::new(
                "files",
                vec![
                    (
                        "protocol",
                        match files.protocol {
                            FilesProtocol::S3 => "s3",
                            FilesProtocol::Sftp => "sftp",
                        }
                        .to_owned(),
                    ),
                    (
                        "endpoint",
                        files
                            .endpoint
                            .as_deref()
                            .map(str::trim)
                            .filter(|endpoint| !endpoint.is_empty())
                            .map(origin_key)
                            .unwrap_or_default(),
                    ),
                    ("bucket", files.bucket.clone()),
                ],
            ),
            Dial::Rest(rest) => {
                TargetIdentity::new("rest", vec![("baseUrl", origin_key(&rest.base_url))])
            }
            Dial::Mongo(mongo) => TargetIdentity::new(
                "mongodb",
                vec![
                    ("hosts", host_port_set_key(&mongo.hosts)),
                    ("database", mongo.database.clone()),
                ],
            ),
            Dial::Kafka(kafka) => TargetIdentity::new(
                "kafka",
                vec![(
                    "bootstrapServers",
                    host_port_set_key(&kafka.bootstrap_servers),
                )],
            ),
            Dial::Sftp(sftp) => TargetIdentity::new(
                "sftp",
                vec![
                    ("host", host_key(&sftp.host)),
                    ("port", sftp.port.to_string()),
                    (
                        "hostKeyFingerprint",
                        sftp.host_key_fingerprint.trim().to_owned(),
                    ),
                ],
            ),
            Dial::Sheets(_) => TargetIdentity::new("sheets", Vec::new()),
        }
    }
}

/// What a [`Dial`] points at: the adapter plus the fields that decide where
/// the connector's credential is sent. Built only by
/// [`Dial::target_identity`]; compared with `==`.
///
/// Holds host names and ports, never a credential (a `dial` cannot carry
/// one: [`Dial::parse`] refuses unknown fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetIdentity {
    adapter: &'static str,
    parts: Vec<(&'static str, String)>,
}

impl TargetIdentity {
    fn new(adapter: &'static str, parts: Vec<(&'static str, String)>) -> Self {
        Self { adapter, parts }
    }

    /// The adapter this identity belongs to.
    #[must_use]
    pub fn adapter(&self) -> &'static str {
        self.adapter
    }
}

/// Whether saving `new` over the stored `(stored_adapter, stored_dial)`
/// re-points the connector (`SEC-14`): its stored credential would go to a
/// target other than the one it was stored for.
///
/// # The first-configuration boundary
///
/// A connector created through `POST /api/connectors` has `adapter = NULL`
/// and `dial = {}` until its first ingest spec is saved (`0033`'s column
/// defaults). Saving that first dial is not a re-point: there is no stored
/// target to move away from, and the console's create flow does exactly
/// this. Anything else that cannot be compared fails closed and counts as a
/// re-point: a stored `adapter` whose `dial` no longer parses for it, or an
/// `adapter = NULL` row whose `dial` is not empty.
///
/// A pre-`0033` row (`adapter = NULL`) dials from its legacy `host` column
/// until it has an ingest spec; that column is guarded separately, by
/// `PATCH /api/connectors/{id}` refusing a `host` change for such a row.
#[must_use]
pub fn is_repoint(
    stored_adapter: Option<&str>,
    stored_dial: &serde_json::Value,
    new: &Dial,
) -> bool {
    let Some(adapter) = stored_adapter else {
        let empty = match stored_dial {
            serde_json::Value::Null => true,
            serde_json::Value::Object(map) => map.is_empty(),
            _ => false,
        };
        return !empty;
    };
    match Dial::parse(adapter, stored_dial) {
        Ok(stored) => stored.target_identity() != new.target_identity(),
        Err(_) => true,
    }
}

fn driver_key(driver: SqlDriver) -> &'static str {
    match driver {
        SqlDriver::Mysql => "mysql",
        SqlDriver::Postgres => "postgres",
        SqlDriver::Mssql => "mssql",
        SqlDriver::Oracle => "oracle",
    }
}

/// A host name as compared: trimmed and lower-cased.
fn host_key(host: &str) -> String {
    host.trim().to_ascii_lowercase()
}

/// Split `authority` (no userinfo) into a lower-cased host and an optional
/// numeric port. An IPv6 literal keeps its brackets. A port that is not a
/// number leaves the whole text as the host, so it still takes part in the
/// comparison.
fn split_authority(authority: &str) -> (String, Option<u16>) {
    let authority = authority.trim().to_ascii_lowercase();
    let port_split = if authority.starts_with('[') {
        authority
            .rfind("]:")
            .map(|index| (&authority[..=index], &authority[index + 2..]))
    } else {
        authority.rsplit_once(':')
    };
    match port_split.and_then(|(host, port)| Some((host, port.parse::<u16>().ok()?))) {
        Some((host, port)) => (host.to_owned(), Some(port)),
        None => (authority, None),
    }
}

/// `scheme://host:port` of a URL, with the scheme's default port filled in
/// and userinfo, path, query and fragment dropped. Text that is not a
/// `scheme://` URL is compared as it is (lower-cased), so it can only ever
/// match itself.
fn origin_key(raw: &str) -> String {
    let raw = raw.trim();
    let Some((scheme, rest)) = raw.split_once("://") else {
        return format!("raw:{}", raw.to_ascii_lowercase());
    };
    let scheme = scheme.to_ascii_lowercase();
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    let host_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_userinfo, host_port)| host_port);
    let (host, port) = split_authority(host_port);
    let default_port = match scheme.as_str() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    match port.or(default_port) {
        Some(port) => format!("{scheme}://{host}:{port}"),
        None => format!("{scheme}://{host}"),
    }
}

/// The `host:port` entries of a `mongodb` or `kafka` dial as one comparable
/// string: order and repeats do not matter, host case does not matter.
fn host_port_set_key(entries: &[String]) -> String {
    let mut keys: Vec<String> = entries
        .iter()
        .map(|entry| match split_authority(entry) {
            (host, Some(port)) => format!("{host}:{port}"),
            (host, None) => host,
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys.join(",")
}

/// Deserialize `raw` into `T`, wrapping a failure as
/// [`IngestSpecError::InvalidDial`] named after `adapter`.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if `raw` does not match `T`'s
/// shape.
fn parse_dial<T: for<'de> Deserialize<'de>>(
    adapter: &str,
    raw: &serde_json::Value,
) -> Result<T, IngestSpecError> {
    serde_json::from_value(raw.clone()).map_err(|source| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source,
    })
}

/// Check `host` via [`validate_hostname`], wrapping a rejection as
/// [`IngestSpecError::InvalidDial`] under the same variant a
/// `deny_unknown_fields` rejection already produces, so callers handle one
/// error shape.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if `host` is not a valid
/// hostname per [`validate_hostname`].
fn check_hostname(adapter: &str, field: &'static str, host: &str) -> Result<(), IngestSpecError> {
    validate_hostname(field, host).map_err(|err| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source: serde::de::Error::custom(err),
    })
}

/// Split a `host:port`-shaped string into its two halves, refusing an
/// entry that does not contain a colon. Mirrors the `rsplit_once(':')`
/// pattern `dagster/dispar_orchestrate/ssrf_guard_mongo.py`'s
/// `resolve_all_seed_hosts` uses — a host with NO colon is rejected at
/// parse time rather than silently defaulting to `27017`, since
/// `rsplit_once`'s no-match result is a `None` we want to surface, not
/// paper over.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if `value` does not contain a
/// `:` (no embedded port).
fn split_host_port<'a>(
    adapter: &str,
    field: &'static str,
    value: &'a str,
) -> Result<(&'a str, &'a str), IngestSpecError> {
    match value.rsplit_once(':') {
        Some((host, port)) => Ok((host, port)),
        None => Err(IngestSpecError::InvalidDial {
            adapter: adapter.to_owned(),
            source: serde::de::Error::custom(format!(
                "{field} entry {value:?} must be host:port-shaped"
            )),
        }),
    }
}

/// Post-deserialize application-level validation for an [`SqlDial`]:
/// the rules serde cannot express declaratively.
///
/// 1. Oracle TLS without `ssl_server_cert_dn`: thin-mode `oracledb`
///    performs the TLS handshake against the connection's `host` field,
///    which the Oracle build's `adapters/oracle.py` deliberately sets to
///    an IP literal (the `resolve_checked` address). A `ssl_server_cert_dn`
///    that names the certificate is therefore REQUIRED whenever
///    `ssl_mode` implies TLS — without it, the driver silently degrades
///    to matching a certificate against an IP, which never matches a
///    real cert's full DN. Refused at save time, never a connection with
///    the server's identity unverified.
/// 2. Control characters in `ssl_server_cert_dn`: the operator-supplied
///    DN is passed to `oracledb` UNCHANGED. A DN with control characters
///    could be an injection attempt; refuse at save time.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] if either rule fails.
fn validate_sql_dial_post_parse(adapter: &str, dial: &SqlDial) -> Result<(), IngestSpecError> {
    if let Some(dn) = dial.ssl_server_cert_dn.as_deref() {
        for byte in dn.bytes() {
            if byte < 0x20 || byte == 0x7f {
                return Err(IngestSpecError::InvalidDial {
                    adapter: adapter.to_owned(),
                    source: serde::de::Error::custom(format!(
                        "sslServerCertDn contains a control character (byte 0x{byte:02x})"
                    )),
                });
            }
        }
    }

    if dial.driver == SqlDriver::Oracle {
        let tls_required = dial
            .ssl_mode
            .as_deref()
            .is_some_and(|mode| mode != "disable");
        if tls_required && dial.ssl_server_cert_dn.as_deref().is_none_or(str::is_empty) {
            return Err(IngestSpecError::InvalidDial {
                adapter: adapter.to_owned(),
                source: serde::de::Error::custom(
                    "sslServerCertDn is required when sslMode requires TLS: the server's \
                     identity cannot be verified when connecting by IP; set sslServerCertDn to \
                     the certificate's expected Distinguished Name, or set sslMode to disable",
                ),
            });
        }
    }

    Ok(())
}

/// A `dial` failed to parse or validate.
///
/// [`IngestSpecError::InvalidDial`] never echoes the raw `dial` JSON —
/// only serde's own field-path message (e.g. "unknown field `password`,
/// expected one of ...") — since `dial` is exactly where a smuggled
/// credential would be (AGENTS.md principle 4, applied to input rather
/// than upstream `sqlx` errors: never surface unclassified raw text).
#[derive(Debug, thiserror::Error)]
pub enum IngestSpecError {
    /// The connector's `adapter` column names a value not in the closed
    /// `sql | cdc | mongodb | kafka | sftp | files | rest | sheets` set.
    #[error("unknown adapter {adapter:?}")]
    UnknownAdapter {
        /// The offending adapter value.
        adapter: String,
    },
    /// `dial` failed to deserialize into the shape `adapter` names, or
    /// failed [`validate_hostname`]. Carries only `source`'s own message
    /// (serde's field-path text, or [`InvalidHostname`]'s message via
    /// `serde::de::Error::custom`) — never the raw JSON passed to
    /// [`Dial::parse`].
    #[error("dial for adapter {adapter:?} is invalid: {source}")]
    InvalidDial {
        /// The adapter whose shape `dial` failed to match.
        adapter: String,
        /// The underlying serde error. Its `Display` never echoes the raw
        /// input value, only the field path and expectation.
        source: serde_json::Error,
    },
}

/// How one run's rows meet the Bronze table they land in. Mirrors
/// `LOAD_MODES` in `dagster/dispar_orchestrate/adapters/sink.py`, which is
/// what acts on it; the wire form is the lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LoadMode {
    /// Overwrite the table with what the source returns now.
    Replace,
    /// Add every row the source returns; two runs leave two copies.
    Append,
    /// Add only rows beyond the cursor column's last seen value.
    Incremental,
}

/// One object (table, endpoint, sheet range) an ingest job targets.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SourceObject {
    /// `"<schema>.<table>"`-shaped for `sql`/`cdc`, or an adapter-specific
    /// name otherwise.
    pub name: String,
    /// The cursor column of an [`LoadMode::Incremental`] object.
    pub incremental_key: Option<String>,
    /// `None` for an object saved before load modes existed; the ingest
    /// job then replaces.
    #[serde(default)]
    pub load_mode: Option<LoadMode>,
    /// The Bronze target this object lands at.
    pub target: String,
}

/// The longest cursor column name accepted, generous for any SQL
/// identifier (`PostgreSQL` 63, `MySQL` 64, SQL Server 128).
const MAX_INCREMENTAL_KEY_LEN: usize = 128;

/// Why a spec's `sourceObjects` cannot be ingested as written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidLoadMode {
    /// `loadMode` is not a [`LoadMode`].
    #[error("source object {name:?} has loadMode {mode:?}; use replace, append or incremental")]
    Unknown {
        /// The source object's name.
        name: String,
        /// The offending value, as sent.
        mode: String,
    },
    /// `incremental` for an adapter the ingest job does not run it for.
    #[error(
        "source object {name:?} asks for loadMode incremental, which only SQL connectors support"
    )]
    NotForAdapter {
        /// The source object's name.
        name: String,
    },
    /// `incremental` without a usable cursor column.
    #[error(
        "source object {name:?} asks for loadMode incremental, which needs incrementalKey: the \
         column that marks new rows"
    )]
    NoCursor {
        /// The source object's name.
        name: String,
    },
}

/// Check each source object's load mode, the one part of `sourceObjects`
/// checked at save time: a mode the ingest job cannot run would otherwise
/// surface only as a rejected run. Everything else in an object is left
/// as it is, as before.
///
/// # Errors
///
/// Returns [`InvalidLoadMode`] for the first object with an unknown
/// `loadMode`, or with `incremental` on a non-`sql` adapter or without a
/// cursor column that is a plain name of sensible length.
pub fn validate_load_modes(
    adapter: &str,
    source_objects: &serde_json::Value,
) -> Result<(), InvalidLoadMode> {
    let Some(objects) = source_objects.as_array() else {
        return Ok(());
    };
    for object in objects {
        let name = object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let mode = match object.get("loadMode") {
            None | Some(serde_json::Value::Null) => continue,
            Some(value) => match LoadMode::deserialize(value) {
                Ok(mode) => mode,
                Err(_) => {
                    return Err(InvalidLoadMode::Unknown {
                        name,
                        mode: value
                            .as_str()
                            .map_or_else(|| value.to_string(), str::to_owned),
                    });
                }
            },
        };
        if mode != LoadMode::Incremental {
            continue;
        }
        if adapter != "sql" {
            return Err(InvalidLoadMode::NotForAdapter { name });
        }
        let cursor = object
            .get("incrementalKey")
            .and_then(serde_json::Value::as_str)
            .map_or("", str::trim);
        if cursor.is_empty()
            || cursor.len() > MAX_INCREMENTAL_KEY_LEN
            || cursor.chars().any(char::is_control)
        {
            return Err(InvalidLoadMode::NoCursor { name });
        }
    }
    Ok(())
}

/// This [`SourceObject`] failed application-level validation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("source object {name:?} has an empty target")]
pub struct InvalidSourceObject {
    /// The offending source object's name.
    pub name: String,
}

impl SourceObject {
    /// Reject a [`SourceObject`] with an empty `target`: an ingest job
    /// with nowhere to land its output is a construction-time error, not
    /// a later runtime surprise.
    ///
    /// # Errors
    ///
    /// Returns [`InvalidSourceObject`] if `target` is empty.
    pub fn validate(&self) -> Result<(), InvalidSourceObject> {
        if self.target.is_empty() {
            Err(InvalidSourceObject {
                name: self.name.clone(),
            })
        } else {
            Ok(())
        }
    }
}

/// `host`/`value` is not a DNS hostname or IP-literal shape safe to
/// interpolate into a delimiter-separated text format.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field} is not a valid hostname: {value:?}")]
pub struct InvalidHostname {
    /// The field name, for error messages only.
    pub field: &'static str,
    /// The rejected value.
    pub value: String,
}

/// A DNS hostname or IP-literal shape safe to interpolate into a
/// delimiter-separated text format (an ODBC connection string, a DSN)
/// without further quoting: RFC 1123 labels (letters/digits/hyphens, no
/// leading/trailing hyphen, 1-63 chars each) separated by `.`, total
/// length <= 253, and — the property THIS rule exists for — no `;`, no
/// `{`/`}`, no control character, since those are exactly the characters
/// an ODBC/DSN-shaped consumer treats specially.
///
/// This is the one canonical hostname rule this workstream cites, so that
/// no interpolation site has to invent its own: `SqlDial`/`CdcDial`'s
/// `host` is checked here, at `dial`-construction time, in Rust; the same
/// rule is ported verbatim to Python by the Dagster `sql` adapter's own
/// hostname check (`dagster/dispar_orchestrate/adapters/sql.py`), which
/// interpolates `host` into an ODBC connection string for `mssql`.
///
/// # Errors
///
/// Returns [`InvalidHostname`] if `host` does not match the shape
/// described above.
pub fn validate_hostname(field: &'static str, host: &str) -> Result<(), InvalidHostname> {
    let labels: Vec<&str> = host.split('.').collect();
    let ok = !host.is_empty()
        && host.len() <= 253
        && !labels.is_empty()
        && labels.iter().all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if ok {
        Ok(())
    } else {
        Err(InvalidHostname {
            field,
            value: host.to_owned(),
        })
    }
}

/// The ONE `(adapter, auth type) -> named secret fields` mapping (WS3
/// plan review Z6), used by [`crate::connectors::set_ingest_spec`] to
/// require the right NUMBER of secret refs for a given adapter/auth-type
/// combination at SAVE time.
///
/// Mirrors `dagster/dispar_orchestrate/secret_map.py`'s
/// `SECRET_FIELD_NAMES` dict EXACTLY -- see that module's doc comment for
/// why this is a same-commit literal pin, not a cross-language import
/// (this workspace has none), and the bug this pin replaces: an earlier
/// Dagster-side revision derived an env-var name from a principal-chosen
/// connector id instead of consulting a named mapping like this one. A
/// Rust-side change to this `match` must land in the same commit as the
/// matching Python-side change (the same discipline `column_gate.py` and
/// `cdc.rs::NESTED_TYPE_MARKERS` already carry, X9).
///
/// `auth_type` is consulted for `adapter == "rest"` (the `dial.auth.type`
/// tag), `adapter == "kafka"` (`"none"` needs zero secret refs — a
/// `PLAINTEXT` broker has no credential at all — while `"sasl_plain"`
/// needs two) and `adapter == "sftp"` (`"password"` vs. `"public_key"`
/// select a different single field name, never a field COUNT, so both
/// stay one-`secretRef`). Every other adapter's field count does not
/// depend on it. Returns `None` for a combination this mapping does not
/// recognize; the caller ([`crate::connectors::set_ingest_spec`]) turns
/// that into [`crate::StoreError::Validation`], never a silent default.
#[must_use]
pub fn secret_field_names(
    adapter: &str,
    auth_type: Option<&str>,
) -> Option<&'static [&'static str]> {
    match (adapter, auth_type) {
        // Every one of these needs exactly the single field `password`,
        // for a different reason each time: sql/cdc/mongodb carry no
        // auth-type distinction at all, sftp's "password" auth type is
        // its one single-field case (its OTHER auth type, public_key,
        // needs a differently-named field below, which is why sftp is
        // not folded into the `_`-wildcard adapters on this arm).
        // Kafka SASL/PLAIN is here too: its username is configuration the
        // dial carries (`KafkaAuth::SaslPlain { username }`, checked by
        // `Dial::parse`), so only the password is a secret.
        ("sql" | "cdc" | "mongodb", _)
        | ("sftp", Some("password"))
        | ("kafka", Some("sasl_plain")) => Some(&["password"]),
        ("files", _) => Some(&["accessKey", "secretKey"]),
        ("rest", Some("api_key")) => Some(&["apiKey"]),
        ("rest", Some("bearer")) => Some(&["token"]),
        ("rest", Some("basic")) => Some(&["username", "password"]),
        ("rest", Some("oauth2_client_credentials")) => Some(&["clientId", "clientSecret"]),
        ("kafka", Some("none")) => Some(&[]),
        ("sftp", Some("public_key")) => Some(&["privateKey"]),
        ("sheets", _) => Some(&["serviceAccountJson"]),
        _ => None,
    }
}

#[cfg(test)]
mod secret_field_tests {
    use super::secret_field_names;

    #[test]
    fn secret_field_names_matches_the_ported_python_mapping() {
        // The Python port (secret_map.py) hardcodes the SAME pairs in its
        // own SECRET_FIELD_NAMES dict -- this test's only job is to be
        // the thing that breaks if this match is ever edited without a
        // matching Python edit in the same commit (X9's discipline).
        assert_eq!(
            secret_field_names("sql", None),
            Some(["password"].as_slice())
        );
        assert_eq!(
            secret_field_names("cdc", None),
            Some(["password"].as_slice())
        );
        assert_eq!(
            secret_field_names("files", None),
            Some(["accessKey", "secretKey"].as_slice())
        );
        assert_eq!(
            secret_field_names("rest", Some("api_key")),
            Some(["apiKey"].as_slice())
        );
        assert_eq!(
            secret_field_names("rest", Some("bearer")),
            Some(["token"].as_slice())
        );
        assert_eq!(
            secret_field_names("rest", Some("basic")),
            Some(["username", "password"].as_slice())
        );
        assert_eq!(
            secret_field_names("rest", Some("oauth2_client_credentials")),
            Some(["clientId", "clientSecret"].as_slice())
        );
        assert_eq!(
            secret_field_names("sheets", None),
            Some(["serviceAccountJson"].as_slice())
        );
        assert_eq!(secret_field_names("rest", Some("not-a-real-type")), None);
        assert_eq!(secret_field_names("smtp", None), None);
    }

    #[test]
    fn secret_field_names_ignores_auth_type_for_non_rest_adapters() {
        assert_eq!(
            secret_field_names("sql", Some("bearer")),
            Some(["password"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_mongodb_needs_only_a_password() {
        assert_eq!(
            secret_field_names("mongodb", None),
            Some(["password"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_oracle_needs_only_a_password_like_every_other_sql_driver() {
        // Oracle is `adapter = "sql"`, `dial.driver = "oracle"` (see
        // `SqlDriver::Oracle`'s own doc comment) — the mapping key is
        // `("sql", None)` regardless of which `SqlDriver` the dial names.
        assert_eq!(
            secret_field_names("sql", None),
            Some(["password"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_kafka_none_auth_needs_no_secret_at_all() {
        assert_eq!(
            secret_field_names("kafka", Some("none")),
            Some([].as_slice())
        );
    }

    #[test]
    fn secret_field_names_kafka_sasl_plain_needs_only_the_password() {
        assert_eq!(
            secret_field_names("kafka", Some("sasl_plain")),
            Some(["password"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_sftp_password_auth_needs_a_password() {
        assert_eq!(
            secret_field_names("sftp", Some("password")),
            Some(["password"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_sftp_public_key_auth_needs_a_private_key() {
        assert_eq!(
            secret_field_names("sftp", Some("public_key")),
            Some(["privateKey"].as_slice())
        );
    }

    #[test]
    fn secret_field_names_kafka_with_no_auth_type_is_an_unrecognized_combination() {
        // Unlike `rest`/`sftp`, whose adapter-level entries used to fall
        // back to `_` (ignoring `auth_type`), `kafka` has no such
        // fallback — every `KafkaAuth` variant is tagged, so `None` here
        // means the caller never consulted `Dial::secret_map_auth_type`,
        // which is itself a bug worth failing loudly on.
        assert_eq!(secret_field_names("kafka", None), None);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    #[test]
    fn sql_dial_parses_the_documented_shape() {
        let json = serde_json::json!({
            "driver": "mysql", "host": "db.internal", "port": 3306,
            "database": "orders", "user": "app_reader", "sslMode": "required"
        });
        let dial: SqlDial = serde_json::from_value(json).expect("valid sql dial");
        assert_eq!(dial.driver, SqlDriver::Mysql);
        assert_eq!(dial.user, "app_reader");
    }

    #[test]
    fn sql_dial_rejects_a_password_field() {
        let json = serde_json::json!({
            "driver": "mysql", "host": "db.internal", "port": 3306,
            "database": "orders", "user": "app_reader", "password": "s3cret"
        });
        let err = serde_json::from_value::<SqlDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn cdc_dial_requires_slot_and_publication_name() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader"
        });
        let err = serde_json::from_value::<CdcDial>(json).unwrap_err();
        assert!(err.to_string().contains("missing field"));
    }

    #[test]
    fn cdc_dial_rejects_a_password_field() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub",
            "password": "s3cret"
        });
        let err = serde_json::from_value::<CdcDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn files_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "protocol": "s3", "bucket": "b", "format": "csv", "accessKey": "AKIA..."
        });
        let err = serde_json::from_value::<FilesDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn rest_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "baseUrl": "https://api.example.com",
            "auth": {"type": "bearer"},
            "pagination": {"type": "none"},
            "endpoints": [],
            "clientSecret": "s3cret"
        });
        let err = serde_json::from_value::<RestDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn sheets_dial_rejects_an_unknown_field() {
        let json = serde_json::json!({
            "spreadsheetId": "abc123", "ranges": ["Sheet1!A1:B2"], "apiKey": "s3cret"
        });
        let err = serde_json::from_value::<SheetsDial>(json).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn dial_parse_dispatches_on_the_adapter_column_value() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader"
        });
        let dial = Dial::parse("sql", &json).expect("valid sql dial");
        assert!(matches!(dial, Dial::Sql(_)));
        assert!(dial.as_cdc().is_none());
    }

    #[test]
    fn dial_parse_rejects_an_unknown_adapter() {
        let err = Dial::parse("smtp", &serde_json::json!({})).unwrap_err();
        assert!(matches!(err, IngestSpecError::UnknownAdapter { adapter } if adapter == "smtp"));
    }

    #[test]
    fn dial_parse_never_echoes_the_raw_dial_json() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader", "password": "s3cret-value"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        let message = err.to_string();
        assert!(!message.contains("s3cret-value"));
        assert!(message.contains("unknown field"));
    }

    #[test]
    fn dial_parse_rejects_a_host_with_a_semicolon_via_invalid_dial() {
        let json = serde_json::json!({
            "driver": "mssql", "host": "db.internal;TrustServerCertificate=yes", "port": 1433,
            "database": "orders", "user": "app_reader"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        assert!(matches!(err, IngestSpecError::InvalidDial { adapter, .. } if adapter == "sql"));
    }

    #[test]
    fn as_cdc_returns_the_cdc_dial_for_a_cdc_adapter() {
        let json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub"
        });
        let dial = Dial::parse("cdc", &json).expect("valid cdc dial");
        assert_eq!(
            dial.as_cdc().map(|cdc| cdc.slot_name.as_str()),
            Some("orders_slot")
        );
    }

    #[test]
    fn source_object_validate_rejects_an_empty_target() {
        let object = SourceObject {
            name: "public.orders".to_owned(),
            incremental_key: None,
            load_mode: None,
            target: String::new(),
        };
        let err = object.validate().unwrap_err();
        assert_eq!(err.name, "public.orders");
    }

    #[test]
    fn source_object_validate_accepts_a_non_empty_target() {
        let object = SourceObject {
            name: "public.orders".to_owned(),
            incremental_key: Some("updated_at".to_owned()),
            load_mode: Some(LoadMode::Incremental),
            target: "bronze.orders".to_owned(),
        };
        assert!(object.validate().is_ok());
    }

    #[test]
    fn source_object_reads_a_load_mode_and_defaults_to_none() {
        let with: SourceObject = serde_json::from_value(serde_json::json!({
            "name": "public.orders", "target": "orders",
            "loadMode": "incremental", "incrementalKey": "updated_at"
        }))
        .expect("valid");
        assert_eq!(with.load_mode, Some(LoadMode::Incremental));
        let without: SourceObject = serde_json::from_value(
            serde_json::json!({ "name": "public.orders", "target": "orders" }),
        )
        .expect("valid");
        assert_eq!(without.load_mode, None);
    }

    #[test]
    fn validate_load_modes_accepts_what_the_ingest_job_can_run() {
        let objects = serde_json::json!([
            { "name": "a", "target": "a" },
            { "name": "b", "target": "b", "loadMode": "replace" },
            { "name": "c", "target": "c", "loadMode": "append" },
            { "name": "d", "target": "d", "loadMode": "incremental", "incrementalKey": "updated_at" },
        ]);
        assert_eq!(validate_load_modes("sql", &objects), Ok(()));
        // Not an array (never saved, or null): nothing to check.
        assert_eq!(validate_load_modes("sql", &serde_json::Value::Null), Ok(()));
    }

    #[test]
    fn validate_load_modes_refuses_what_would_only_fail_at_run_time() {
        let check = |adapter: &str, object: serde_json::Value| {
            validate_load_modes(adapter, &serde_json::json!([object]))
        };
        assert!(matches!(
            check("sql", serde_json::json!({ "name": "a", "target": "a", "loadMode": "merge" })),
            Err(InvalidLoadMode::Unknown { mode, .. }) if mode == "merge"
        ));
        assert!(matches!(
            check(
                "sql",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental" })
            ),
            Err(InvalidLoadMode::NoCursor { .. })
        ));
        assert!(matches!(
            check(
                "sql",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental", "incrementalKey": "  " })
            ),
            Err(InvalidLoadMode::NoCursor { .. })
        ));
        assert!(matches!(
            check(
                "rest",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental", "incrementalKey": "id" })
            ),
            Err(InvalidLoadMode::NotForAdapter { .. })
        ));
    }

    #[test]
    fn validate_hostname_rejects_a_semicolon_and_braces() {
        assert!(validate_hostname("host", "db.internal;TrustServerCertificate=yes").is_err());
        assert!(validate_hostname("host", "db{internal}").is_err());
        assert!(validate_hostname("host", "db.internal").is_ok());
    }

    #[test]
    fn validate_hostname_rejects_a_leading_hyphen_label() {
        assert!(validate_hostname("host", "-bad.internal").is_err());
    }

    #[test]
    fn validate_hostname_rejects_an_empty_host() {
        assert!(validate_hostname("host", "").is_err());
    }

    // ── Tier 2 adapter shapes ───────────────────────────────────────────

    #[test]
    fn mongo_dial_parses_a_plain_uri_and_rejects_srv() {
        let json = serde_json::json!({
            "hosts": ["mongo-a.internal:27017", "mongo-b.internal:27017"],
            "database": "catalog",
            "username": "reader",
            "directConnection": true
        });
        let dial: MongoDial =
            serde_json::from_value(json).expect("valid mongo dial without srvUri parses");
        assert_eq!(dial.hosts.len(), 2);
        assert!(dial.direct_connection);

        let with_srv = serde_json::json!({
            "srvUri": "mongodb+srv://cluster.example.net/",
            "hosts": ["mongo-a.internal:27017"],
            "database": "catalog",
            "username": "reader",
            "directConnection": true
        });
        let err = serde_json::from_value::<MongoDial>(with_srv).unwrap_err();
        assert!(
            err.to_string().contains("unknown field"),
            "srvUri must be refused by deny_unknown_fields, got: {err}"
        );
    }

    #[test]
    fn mongo_dial_has_no_srv_uri_field_at_all() {
        // "Refuse mongodb+srv and replica-set discovery outright":
        // `deny_unknown_fields` is the structural guard, not a runtime
        // check. Any MongoDial JSON carrying `srvUri` (or any other
        // field not on the closed struct) fails to deserialize.
        let extra = serde_json::json!({
            "hosts": ["m.internal:27017"],
            "database": "d",
            "username": "u",
            "directConnection": true,
            "srvUri": "mongodb+srv://x/"
        });
        let err = serde_json::from_value::<MongoDial>(extra).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
    }

    #[test]
    fn mongo_dial_requires_direct_connection_true() {
        let json = serde_json::json!({
            "hosts": ["m.internal:27017"],
            "database": "d",
            "username": "u",
            "directConnection": false
        });
        // `directConnection` is a `bool` field — `false` parses. The
        // Dagster side (`ssrf_guard_mongo.validate_mongo_dial`) refuses
        // `false` at dial time; this test pins the Rust schema accepts
        // both values (the refusal is NOT a structural property of the
        // schema, by design — see the module doc comment on MongoDial).
        let dial: MongoDial = serde_json::from_value(json).expect("false is parseable");
        assert!(!dial.direct_connection);
    }

    #[test]
    fn kafka_dial_parses_bootstrap_servers_and_auth() {
        let json = serde_json::json!({
            "bootstrapServers": ["broker-a.internal:9092", "broker-b.internal:9092"],
            "topic": "orders",
            "auth": {"type": "sasl_plain", "username": "orders-reader"},
            "groupId": "lakehouse-orders-consumer",
            "microBatchSeconds": 30
        });
        let dial: KafkaDial = serde_json::from_value(json).expect("valid kafka dial");
        assert_eq!(dial.bootstrap_servers.len(), 2);
        assert_eq!(dial.topic, "orders");
        assert_eq!(dial.group_id, "lakehouse-orders-consumer");
        assert_eq!(dial.micro_batch_seconds, 30);
        assert_eq!(dial.auth.type_tag(), "sasl_plain");
        assert_eq!(
            dial.auth,
            KafkaAuth::SaslPlain {
                username: "orders-reader".to_owned()
            }
        );
    }

    #[test]
    fn secret_map_auth_type_reads_the_kafka_auth_tag_through_the_dial_enum() {
        let dial = Dial::Kafka(KafkaDial {
            bootstrap_servers: vec!["broker.internal:9092".to_owned()],
            topic: "orders".to_owned(),
            auth: KafkaAuth::SaslPlain {
                username: "orders-reader".to_owned(),
            },
            group_id: "lakehouse-orders-consumer".to_owned(),
            micro_batch_seconds: 30,
        });
        assert_eq!(dial.secret_map_auth_type(), Some("sasl_plain"));
    }

    #[test]
    fn secret_map_auth_type_reads_the_sftp_auth_tag_through_the_dial_enum() {
        let dial = Dial::Sftp(SftpDial {
            host: "sftp.internal".to_owned(),
            port: 22,
            user: "lakehouse".to_owned(),
            host_key_fingerprint: "SHA256:deadbeef".to_owned(),
            path: "/outbox".to_owned(),
            file_format: "csv".to_owned(),
            auth: SftpAuth::PublicKey,
        });
        assert_eq!(dial.secret_map_auth_type(), Some("public_key"));
    }

    #[test]
    fn secret_map_auth_type_is_none_for_a_dial_with_no_auth_type_at_all() {
        let dial = Dial::Mongo(MongoDial {
            hosts: vec!["mongo.internal:27017".to_owned()],
            database: "app".to_owned(),
            username: "reader".to_owned(),
            direct_connection: true,
        });
        assert_eq!(dial.secret_map_auth_type(), None);
    }

    #[test]
    fn sftp_dial_requires_a_host_key_fingerprint() {
        let without_fingerprint = serde_json::json!({
            "host": "sftp.bank.example",
            "port": 22,
            "user": "lakehouse",
            "path": "/outbox",
            "fileFormat": "csv",
            "auth": {"type": "password"}
        });
        let err = serde_json::from_value::<SftpDial>(without_fingerprint).unwrap_err();
        assert!(
            err.to_string().contains("missing field"),
            "hostKeyFingerprint must be required, got: {err}"
        );

        let with_fingerprint_json = serde_json::json!({
            "host": "sftp.bank.example",
            "port": 22,
            "user": "lakehouse",
            "hostKeyFingerprint": "SHA256:base64==",
            "path": "/outbox",
            "fileFormat": "csv",
            "auth": {"type": "password"}
        });
        let with_fingerprint: SftpDial =
            serde_json::from_value(with_fingerprint_json).expect("with fingerprint parses");
        assert_eq!(with_fingerprint.host_key_fingerprint, "SHA256:base64==");
        assert_eq!(with_fingerprint.auth, SftpAuth::Password);
    }

    #[test]
    fn sql_driver_gains_an_oracle_variant() {
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle-gl.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader"
        });
        let dial: SqlDial =
            serde_json::from_value(json).expect("oracle driver parses as SqlDriver::Oracle");
        assert_eq!(dial.driver, SqlDriver::Oracle);
    }

    #[test]
    fn sql_dial_accepts_an_explicit_ssl_server_cert_dn() {
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle-gl.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader",
            "sslMode": "required",
            "sslServerCertDn": "CN=oracle-gl.internal,OU=Finance,O=Acme,C=US"
        });
        let dial: SqlDial = serde_json::from_value(json).expect("oracle dial with DN parses");
        assert_eq!(
            dial.ssl_server_cert_dn.as_deref(),
            Some("CN=oracle-gl.internal,OU=Finance,O=Acme,C=US")
        );
    }

    #[test]
    fn sql_dial_rejects_a_control_character_in_ssl_server_cert_dn() {
        // Control character in the DN: the operator-supplied DN is
        // passed to oracledb unchanged, so a control char is an
        // injection vector and refused at save time.
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader",
            "sslMode": "required",
            "sslServerCertDn": "CN=oracle.internal\x00,OU=Finance"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("control character"),
            "control character must be refused at Dial::parse, got: {msg}"
        );
    }

    #[test]
    fn dial_parse_refuses_oracle_tls_with_no_ssl_server_cert_dn() {
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader",
            "sslMode": "required"
        });
        let err = Dial::parse("sql", &json).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("cannot be verified when connecting by IP"),
            "Oracle TLS without DN must mention the IP-vs-DN mismatch, got: {msg}"
        );
    }

    #[test]
    fn dial_parse_accepts_oracle_tls_with_an_explicit_ssl_server_cert_dn() {
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader",
            "sslMode": "required",
            "sslServerCertDn": "CN=oracle.internal,OU=Finance,O=Acme,C=US"
        });
        let dial = Dial::parse("sql", &json).expect("Oracle TLS with explicit DN is accepted");
        assert!(matches!(dial, Dial::Sql(_)));
    }

    #[test]
    fn dial_parse_allows_oracle_plaintext_with_no_ssl_server_cert_dn() {
        // Plaintext (no sslMode): no DN required, parses cleanly.
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader"
        });
        let dial = Dial::parse("sql", &json).expect("Oracle plaintext without sslMode is accepted");
        assert!(matches!(dial, Dial::Sql(_)));

        // sslMode = "disable" is also plaintext-shaped.
        let json = serde_json::json!({
            "driver": "oracle",
            "host": "oracle.internal",
            "port": 1521,
            "database": "GL",
            "user": "reader",
            "sslMode": "disable"
        });
        let dial = Dial::parse("sql", &json).expect("Oracle with sslMode=disable is accepted");
        assert!(matches!(dial, Dial::Sql(_)));
    }

    #[test]
    fn dial_parse_rejects_a_kafka_bootstrap_server_shaped_for_injection() {
        let json = serde_json::json!({
            "bootstrapServers": ["broker;evil=1:9092"],
            "topic": "orders",
            "auth": {"type": "sasl_plain", "username": "u"},
            "groupId": "g",
            "microBatchSeconds": 30
        });
        let err = Dial::parse("kafka", &json).unwrap_err();
        assert!(
            matches!(&err, IngestSpecError::InvalidDial { adapter, .. } if adapter == "kafka"),
            "validate_hostname must reject injection-shaped bootstrap, got: {err}"
        );
    }

    #[test]
    fn dial_parse_rejects_a_mongo_host_with_braces() {
        let json = serde_json::json!({
            "hosts": ["mongo{a}.internal:27017"],
            "database": "catalog",
            "username": "reader",
            "directConnection": true
        });
        let err = Dial::parse("mongodb", &json).unwrap_err();
        assert!(
            matches!(&err, IngestSpecError::InvalidDial { adapter, .. } if adapter == "mongodb"),
            "validate_hostname must reject brace-shaped host, got: {err}"
        );
    }

    #[test]
    fn dial_parse_dispatches_the_three_new_adapters() {
        // Pinpoint the dispatch shape on `adapter` for the three new
        // adapters (Mongo/Kafka/Sftp): each lands on its own variant,
        // distinct from the existing five.
        let mongo = serde_json::json!({
            "hosts": ["m.internal:27017"],
            "database": "d",
            "username": "u",
            "directConnection": true
        });
        let parsed = Dial::parse("mongodb", &mongo).expect("mongodb parses");
        assert!(matches!(parsed, Dial::Mongo(_)));

        let kafka = serde_json::json!({
            "bootstrapServers": ["b.internal:9092"],
            "topic": "t",
            "auth": {"type": "none"},
            "groupId": "g",
            "microBatchSeconds": 60
        });
        let parsed = Dial::parse("kafka", &kafka).expect("kafka parses");
        assert!(matches!(parsed, Dial::Kafka(_)));

        let sftp = serde_json::json!({
            "host": "sftp.internal",
            "port": 22,
            "user": "u",
            "hostKeyFingerprint": "SHA256:x",
            "path": "/inbox",
            "fileFormat": "csv",
            "auth": {"type": "public_key"}
        });
        let parsed = Dial::parse("sftp", &sftp).expect("sftp parses");
        assert!(matches!(parsed, Dial::Sftp(_)));
    }

    #[test]
    fn mysql_dial_round_trips_without_ssl_server_cert_dn() {
        // The new field is `#[serde(default)]` so existing `mysql`/`mssql`
        // dials without it continue to deserialize (round-tripping
        // through `serde_json::to_value` would require `Serialize`,
        // which is intentionally NOT derived on these dial structs --
        // see the `Dial::parse` doc comment's "never echo the raw
        // `dial` JSON" rationale).
        let json = serde_json::json!({
            "driver": "mysql",
            "host": "db.internal",
            "port": 3306,
            "database": "orders",
            "user": "app_reader",
            "sslMode": "required"
        });
        let dial: SqlDial = serde_json::from_value(json).expect("mysql parses");
        assert!(dial.ssl_server_cert_dn.is_none());
        assert_eq!(dial.driver, SqlDriver::Mysql);
    }
}

/// `SEC-14` task 1: [`Dial::target_identity`] and [`is_repoint`].
#[cfg(test)]
mod target_identity_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use serde_json::{Value, json};

    use super::{Dial, is_repoint};

    fn parse(adapter: &str, dial: &Value) -> Dial {
        Dial::parse(adapter, dial).expect("a valid dial")
    }

    /// `base` with `field` set to `value`, parsed for `adapter`.
    fn with(adapter: &str, base: &Value, field: &str, value: Value) -> Dial {
        let mut changed = base.clone();
        changed[field] = value;
        parse(adapter, &changed)
    }

    fn same(adapter: &str, base: &Value, field: &str, value: Value) -> bool {
        parse(adapter, base).target_identity()
            == with(adapter, base, field, value).target_identity()
    }

    fn sql() -> Value {
        json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "reader"
        })
    }

    #[test]
    fn a_sql_dial_is_re_pointed_by_driver_host_port_or_database() {
        let base = sql();
        assert!(!same("sql", &base, "driver", json!("mysql")));
        assert!(!same("sql", &base, "host", json!("other.internal")));
        assert!(!same("sql", &base, "port", json!(5433)));
        assert!(!same("sql", &base, "database", json!("billing")));
    }

    #[test]
    fn a_sql_dial_keeps_its_target_when_user_or_ssl_settings_change() {
        let base = sql();
        assert!(same("sql", &base, "user", json!("someone_else")));
        assert!(same("sql", &base, "sslMode", json!("require")));
        assert!(same("sql", &base, "sslServerCertDn", json!("CN=db")));
    }

    #[test]
    fn host_names_compare_case_insensitively() {
        assert!(same("sql", &sql(), "host", json!("DB.Internal")));
    }

    #[test]
    fn a_sql_port_is_always_explicit_so_a_different_port_is_a_different_target() {
        // The shape has no absent port: there is no default to compare
        // against, and the usual port is not treated as one.
        assert!(!same("sql", &sql(), "port", json!(5433)));
        let missing = json!({
            "driver": "postgres", "host": "db.internal",
            "database": "orders", "user": "reader"
        });
        assert!(Dial::parse("sql", &missing).is_err());
    }

    #[test]
    fn a_cdc_dial_is_re_pointed_by_driver_host_port_or_database_only() {
        let base = json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "reader",
            "slotName": "slot_a", "publicationName": "pub_a"
        });
        assert!(!same("cdc", &base, "driver", json!("mysql")));
        assert!(!same("cdc", &base, "host", json!("other.internal")));
        assert!(!same("cdc", &base, "port", json!(5433)));
        assert!(!same("cdc", &base, "database", json!("billing")));
        assert!(same("cdc", &base, "user", json!("someone_else")));
        assert!(same("cdc", &base, "slotName", json!("slot_b")));
        assert!(same("cdc", &base, "publicationName", json!("pub_b")));
        assert!(same("cdc", &base, "serverId", json!(7)));
    }

    #[test]
    fn changing_the_adapter_is_a_re_point_even_on_the_same_host() {
        let cdc = json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "reader",
            "slotName": "s", "publicationName": "p"
        });
        assert_ne!(
            parse("sql", &sql()).target_identity(),
            parse("cdc", &cdc).target_identity()
        );
    }

    fn files() -> Value {
        json!({
            "protocol": "s3", "endpoint": "https://store.internal:9000",
            "bucket": "landing", "format": "csv"
        })
    }

    #[test]
    fn a_files_dial_is_re_pointed_by_protocol_endpoint_or_bucket() {
        let base = files();
        assert!(!same("files", &base, "protocol", json!("sftp")));
        assert!(!same(
            "files",
            &base,
            "endpoint",
            json!("https://other.internal:9000")
        ));
        assert!(!same(
            "files",
            &base,
            "endpoint",
            json!("http://store.internal:9000")
        ));
        assert!(!same(
            "files",
            &base,
            "endpoint",
            json!("https://store.internal:9001")
        ));
        assert!(!same("files", &base, "bucket", json!("other")));
    }

    #[test]
    fn a_files_dial_keeps_its_target_when_prefix_format_or_region_change() {
        let base = files();
        assert!(same("files", &base, "prefix", json!("2026/")));
        assert!(same("files", &base, "format", json!("parquet")));
        assert!(same("files", &base, "region", json!("eu-west-1")));
    }

    #[test]
    fn a_files_endpoint_is_compared_by_origin_with_the_default_port_applied() {
        let base = json!({
            "protocol": "s3", "endpoint": "https://store.internal",
            "bucket": "landing", "format": "csv"
        });
        assert!(same(
            "files",
            &base,
            "endpoint",
            json!("https://store.internal:443")
        ));
        assert!(same(
            "files",
            &base,
            "endpoint",
            json!("HTTPS://Store.Internal/")
        ));
        assert!(same(
            "files",
            &base,
            "endpoint",
            json!("https://store.internal/some/path")
        ));
        assert!(!same(
            "files",
            &base,
            "endpoint",
            json!("https://store.internal:80")
        ));
    }

    #[test]
    fn a_files_dial_with_no_endpoint_differs_from_one_that_names_an_endpoint() {
        let none = json!({ "protocol": "s3", "bucket": "landing", "format": "csv" });
        assert!(!same(
            "files",
            &none,
            "endpoint",
            json!("https://store.internal")
        ));
        // An empty endpoint is the same as none.
        assert!(same("files", &none, "endpoint", json!("  ")));
    }

    fn rest() -> Value {
        json!({
            "baseUrl": "https://api.example.com/v1",
            "auth": { "type": "bearer" },
            "pagination": { "type": "none" },
            "endpoints": []
        })
    }

    #[test]
    fn a_rest_dial_is_re_pointed_by_scheme_host_or_port_of_the_base_url() {
        let base = rest();
        assert!(!same(
            "rest",
            &base,
            "baseUrl",
            json!("http://api.example.com/v1")
        ));
        assert!(!same(
            "rest",
            &base,
            "baseUrl",
            json!("https://api.other.com/v1")
        ));
        assert!(!same(
            "rest",
            &base,
            "baseUrl",
            json!("https://api.example.com:8443/v1")
        ));
    }

    #[test]
    fn a_rest_base_url_default_port_is_the_same_target_and_other_paths_too() {
        let base = rest();
        assert!(same(
            "rest",
            &base,
            "baseUrl",
            json!("https://api.example.com:443/v1")
        ));
        assert!(same(
            "rest",
            &base,
            "baseUrl",
            json!("https://API.example.com")
        ));
        assert!(same(
            "rest",
            &base,
            "baseUrl",
            json!("https://api.example.com/v2/orders?x=1")
        ));
        let http = json!({
            "baseUrl": "http://api.example.com",
            "auth": { "type": "bearer" },
            "pagination": { "type": "none" },
            "endpoints": []
        });
        assert!(same(
            "rest",
            &http,
            "baseUrl",
            json!("http://api.example.com:80")
        ));
        assert!(!same(
            "rest",
            &http,
            "baseUrl",
            json!("http://api.example.com:443")
        ));
    }

    #[test]
    fn a_rest_base_url_userinfo_does_not_hide_the_host() {
        let base = rest();
        assert!(!same(
            "rest",
            &base,
            "baseUrl",
            json!("https://api.example.com@evil.example/")
        ));
        assert!(same(
            "rest",
            &base,
            "baseUrl",
            json!("https://someone@api.example.com/")
        ));
    }

    #[test]
    fn a_rest_dial_keeps_its_target_when_auth_pagination_or_endpoints_change() {
        let base = rest();
        assert!(same("rest", &base, "auth", json!({ "type": "basic" })));
        assert!(same(
            "rest",
            &base,
            "pagination",
            json!({ "type": "page", "param": "p" })
        ));
        assert!(same(
            "rest",
            &base,
            "endpoints",
            json!([{ "path": "/orders", "recordsPath": null }])
        ));
    }

    fn mongo() -> Value {
        json!({
            "hosts": ["m1.internal:27017", "m2.internal:27017"],
            "database": "shop", "username": "reader", "directConnection": true
        })
    }

    #[test]
    fn a_mongodb_dial_is_re_pointed_by_its_host_set_or_database() {
        let base = mongo();
        assert!(!same(
            "mongodb",
            &base,
            "hosts",
            json!(["m1.internal:27017"])
        ));
        assert!(!same(
            "mongodb",
            &base,
            "hosts",
            json!(["m1.internal:27017", "m3.internal:27017"])
        ));
        assert!(!same(
            "mongodb",
            &base,
            "hosts",
            json!(["m1.internal:27018", "m2.internal:27017"])
        ));
        assert!(!same("mongodb", &base, "database", json!("billing")));
    }

    #[test]
    fn a_mongodb_dial_keeps_its_target_for_order_case_and_username() {
        let base = mongo();
        assert!(same(
            "mongodb",
            &base,
            "hosts",
            json!(["M2.internal:27017", "m1.internal:27017"])
        ));
        assert!(same("mongodb", &base, "username", json!("someone_else")));
    }

    fn kafka() -> Value {
        json!({
            "bootstrapServers": ["k1.internal:9092", "k2.internal:9092"],
            "topic": "orders", "auth": { "type": "none" },
            "groupId": "g", "microBatchSeconds": 30
        })
    }

    #[test]
    fn a_kafka_dial_is_re_pointed_by_its_bootstrap_servers_only() {
        let base = kafka();
        assert!(!same(
            "kafka",
            &base,
            "bootstrapServers",
            json!(["k1.internal:9092"])
        ));
        assert!(!same(
            "kafka",
            &base,
            "bootstrapServers",
            json!(["k1.internal:9093", "k2.internal:9092"])
        ));
        assert!(same(
            "kafka",
            &base,
            "bootstrapServers",
            json!(["K2.internal:9092", "k1.internal:9092"])
        ));
        assert!(same("kafka", &base, "topic", json!("other")));
        assert!(same("kafka", &base, "groupId", json!("other")));
        assert!(same("kafka", &base, "microBatchSeconds", json!(60)));
        assert!(same(
            "kafka",
            &base,
            "auth",
            json!({ "type": "sasl_plain", "username": "u" })
        ));
    }

    fn sftp() -> Value {
        json!({
            "host": "sftp.internal", "port": 22, "user": "reader",
            "hostKeyFingerprint": "SHA256:abc", "path": "/inbox",
            "fileFormat": "csv", "auth": { "type": "password" }
        })
    }

    #[test]
    fn an_sftp_dial_is_re_pointed_by_host_port_or_host_key() {
        let base = sftp();
        assert!(!same("sftp", &base, "host", json!("other.internal")));
        assert!(!same("sftp", &base, "port", json!(2222)));
        assert!(!same(
            "sftp",
            &base,
            "hostKeyFingerprint",
            json!("SHA256:xyz")
        ));
        assert!(same("sftp", &base, "host", json!("SFTP.internal")));
        assert!(same("sftp", &base, "user", json!("someone_else")));
        assert!(same("sftp", &base, "path", json!("/outbox")));
        assert!(same("sftp", &base, "fileFormat", json!("json")));
        assert!(same("sftp", &base, "auth", json!({ "type": "public_key" })));
    }

    #[test]
    fn a_sheets_dial_has_no_target_to_change() {
        let base = json!({ "spreadsheetId": "abc", "ranges": ["A1:B2"] });
        assert!(same("sheets", &base, "spreadsheetId", json!("def")));
        assert!(same("sheets", &base, "ranges", json!(["C1:D2"])));
    }

    #[test]
    fn the_first_configuration_of_a_connector_is_not_a_re_point() {
        let new = parse("sql", &sql());
        assert!(!is_repoint(None, &json!({}), &new));
        assert!(!is_repoint(None, &Value::Null, &new));
    }

    #[test]
    fn a_stored_target_that_cannot_be_compared_counts_as_a_re_point() {
        let new = parse("sql", &sql());
        // No adapter, but a dial: unknown shape.
        assert!(is_repoint(None, &sql(), &new));
        // An adapter whose stored dial no longer parses for it.
        assert!(is_repoint(Some("sql"), &json!({}), &new));
        assert!(is_repoint(Some("rest"), &sql(), &new));
    }

    #[test]
    fn a_stored_target_is_compared_with_the_new_one() {
        let stored = sql();
        assert!(!is_repoint(Some("sql"), &stored, &parse("sql", &stored)));
        assert!(!is_repoint(
            Some("sql"),
            &stored,
            &with("sql", &stored, "user", json!("someone_else"))
        ));
        assert!(is_repoint(
            Some("sql"),
            &stored,
            &with("sql", &stored, "host", json!("other.internal"))
        ));
        assert!(is_repoint(Some("sql"), &stored, &parse("rest", &rest())));
    }
}
