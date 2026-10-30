//! Per-adapter `dial` validation for registry-driven ingestion
//! (`docs/adr/0013-registry-driven-ingestion.md`).
//!
//! `connector.dial` (`0033_connector_ingest_spec.sql`) is free-form
//! `JSONB` at the column level, but never free-form at the application
//! level: [`Dial::parse`] parses it into exactly one of nine
//! `#[serde(deny_unknown_fields, rename_all = "camelCase")]` structs,
//! dispatched on the connector's own `adapter` column value. Every one of
//! the nine structs rejects an unknown field, which is the property that
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
/// Which of `slot_name`, `publication_name` and `server_id` a dial carries
/// depends on the driver (`SRC-4` slice 3, decision S3-a), checked by
/// [`Dial::parse`] (`validate_cdc_dial_post_parse`), not by serde: `PostgreSQL`
/// requires `slotName` and `publicationName` and refuses `serverId`; `MySQL`
/// requires `serverId` and refuses the other two; `SQL Server` refuses all
/// three. A `cdc` dial that breaks this is a construction-time (400) error, not
/// a later runtime surprise. ADR 0013's "required" is "required for
/// `PostgreSQL`" (dated note there).
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
    /// The replication slot name: required for `PostgreSQL` with no fallback,
    /// refused for the other drivers.
    #[serde(default)]
    pub slot_name: Option<String>,
    /// The publication name: required for `PostgreSQL` with no fallback,
    /// refused for the other drivers.
    #[serde(default)]
    pub publication_name: Option<String>,
    /// The `MySQL` binlog client's server id (1 to 4294967295): required for
    /// `MySQL`, refused for the other drivers.
    #[serde(default)]
    pub server_id: Option<u32>,
    /// How the change-capture worker secures its connection to the source
    /// (`SRC-4` decision S2-d). Absent means [`CdcSslMode::Require`]: a
    /// stored credential is never dialed in plain text unless the person
    /// chose `disable`, which the console shows as such (`SEC-14`'s rule).
    #[serde(default)]
    pub ssl_mode: CdcSslMode,
}

/// The TLS modes a change-capture `dial` may name: the four a person can
/// choose, a closed set so a typo fails at parse time. Not `prefer` or
/// `allow` (attempt TLS, fall back to plain text): those would dial a stored
/// credential in the clear without anyone having said so.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CdcSslMode {
    /// No TLS. Only when the person chose it.
    Disable,
    /// Encrypt and take the server certificate as presented.
    #[default]
    Require,
    /// Encrypt and verify the certificate chain.
    VerifyCa,
    /// Encrypt, verify the chain and that the certificate names the host.
    VerifyFull,
}

impl CdcSslMode {
    /// The value `Debezium`'s `database.sslmode` takes for this mode (the
    /// same four words, `PostgreSQL` JDBC's own spelling).
    #[must_use]
    pub const fn as_debezium(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }
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
        ///
        /// `SRC-12` part C: the wire name is `tokenUrl`, the one the console
        /// and `adapters/rest.py` use. The enum-level `rename_all` renames
        /// variants, not the fields of a variant, so without this rename the
        /// field only parsed as `token_url` and a dial written by the console
        /// was refused. `token_url` stays accepted for a row written that way.
        #[serde(rename = "tokenUrl", alias = "token_url")]
        token_url: String,
    },
    /// `OAuth2` authorization-code grant with PKCE (`S256`; `SRC-12` part C,
    /// decision 6). A person signs in at the provider once; the refresh token
    /// the provider returns is the connector's secondary credential (sealed,
    /// part A) and the client secret its primary one (`secret_field_names`).
    ///
    /// Every address here is configuration, not a secret. `token_url` and
    /// `revocation_url` are dialled by the product and receive the client
    /// secret and the refresh token, so they are part of the target identity
    /// ([`Dial::target_identity`]); `authorize_url` is only handed to the
    /// person's browser.
    #[serde(rename = "oauth2_authorization_code")]
    Oauth2AuthorizationCode {
        /// The app's client id (public by the protocol's definition).
        #[serde(rename = "clientId")]
        client_id: String,
        /// The provider's sign-in page. Opened by the browser, never dialled
        /// by the product.
        #[serde(rename = "authorizeUrl")]
        authorize_url: String,
        /// The provider's token endpoint.
        #[serde(rename = "tokenUrl")]
        token_url: String,
        /// The provider's revocation endpoint (RFC 7009), when it has one.
        #[serde(rename = "revocationUrl", default)]
        revocation_url: Option<String>,
        /// The scopes asked for, each one a single token.
        #[serde(default)]
        scopes: Vec<String>,
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
            RestAuth::Oauth2AuthorizationCode { .. } => "oauth2_authorization_code",
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

/// How a `saas` connector signs in (`SRC-3` decision 3), internally tagged
/// on `type`.
///
/// Neither shape carries an address: the sign-in, token and revocation
/// addresses come from the vendor table ([`crate::saas_vendors`], AD8), so a
/// dial cannot send the client secret or the refresh token anywhere the
/// table does not name.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase", tag = "type")]
pub enum SaasAuth {
    /// Sign-in in the browser: `OAuth2` authorisation code with PKCE
    /// (`SRC-12`). The client secret and the refresh token are the
    /// connector's two secrets.
    #[serde(rename = "oauth2_authorization_code")]
    Oauth2AuthorizationCode {
        /// The app's client id (public by the protocol's definition).
        #[serde(rename = "clientId")]
        client_id: String,
    },
    /// The vendor's integration credential: its secret fields are named by
    /// the vendor's table row. A vendor without one refuses this shape.
    #[serde(rename = "integration")]
    Integration,
}

/// The connection shape for a `saas` adapter's `dial` (`SRC-3`, plan AD1).
///
/// One adapter for the ten business-app vendors; `vendor` picks the row of
/// [`crate::saas_vendors::SAAS_VENDORS`] and `account` is the one part of
/// the vendor's address a person supplies (AD2).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SaasDial {
    /// The vendor's id in the table.
    pub vendor: String,
    /// The vendor's per-customer part of the address, in the shape the
    /// vendor's [`crate::saas_vendors::AccountRule`] allows.
    pub account: String,
    /// The objects the person defined by name where the vendor has no
    /// listing of them (reports, for example). Each name is data the
    /// vendor module interprets, never an address.
    #[serde(default)]
    pub objects: Vec<String>,
    /// How the connector signs in.
    pub auth: SaasAuth,
    /// `NetSuite` only (`SRC-3` slice 6): the id of the certificate uploaded in `NetSuite`'s "OAuth 2.0
    /// Client Credentials (M2M) Setup", which goes into the signed assertion's `kid` header. It is a
    /// name, not a secret and not a destination, so it is not part of the target identity.
    #[serde(default)]
    pub certificate_id: Option<String>,
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
    /// A `saas` adapter's dial (`SRC-3`).
    Saas(SaasDial),
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
    /// of `sql`/`cdc`/`mongodb`/`kafka`/`sftp`/`files`/`rest`/`sheets`/`saas`,
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
                validate_cdc_dial_post_parse(adapter, &dial)?;
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
            "rest" => {
                let dial: RestDial = parse_dial(adapter, raw)?;
                validate_rest_dial_post_parse(adapter, &dial)?;
                Ok(Dial::Rest(dial))
            }
            "sheets" => Ok(Dial::Sheets(parse_dial(adapter, raw)?)),
            "saas" => {
                let dial: SaasDial = parse_dial(adapter, raw)?;
                validate_saas_dial_post_parse(adapter, &dial)?;
                Ok(Dial::Saas(dial))
            }
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

    /// The `saas` adapter's `auth_type` key: the browser sign-in's tag, or
    /// the vendor's `integration:<id>` key. `None` for any other adapter
    /// and for an unknown vendor or an integration credential the vendor
    /// does not have ([`Dial::parse`] refuses both, so a parsed dial always
    /// has a key).
    #[must_use]
    pub fn saas_auth_type(&self) -> Option<&'static str> {
        let Dial::Saas(saas) = self else {
            return None;
        };
        let vendor = crate::saas_vendors::vendor(&saas.vendor)?;
        vendor.auth_type_key(matches!(
            saas.auth,
            SaasAuth::Oauth2AuthorizationCode { .. }
        ))
    }

    /// The `auth_type` key [`secret_field_names`] needs for THIS dial's
    /// adapter, whichever of [`Self::rest_auth_type`],
    /// [`Self::kafka_auth_type`], [`Self::sftp_auth_type`] or
    /// [`Self::saas_auth_type`] applies (at most one ever returns `Some`,
    /// since each matches a disjoint set of `Dial` variants) — the single
    /// call [`crate::connectors::set_ingest_spec`] makes instead of trying
    /// each accessor itself.
    #[must_use]
    pub fn secret_map_auth_type(&self) -> Option<&'static str> {
        self.rest_auth_type()
            .or_else(|| self.kafka_auth_type())
            .or_else(|| self.sftp_auth_type())
            .or_else(|| self.saas_auth_type())
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
    /// | `saas` | `vendor`, `account` (the vendor table builds every address from them, `SRC-3` AD2) |
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
                let mut parts = vec![("baseUrl", origin_key(&rest.base_url))];
                // `SRC-12` part C: the token and revocation addresses receive
                // the client secret and the refresh token, so moving either to
                // another host is a re-point exactly as moving `baseUrl` is
                // (`SEC-14`). A client-credentials connector's `tokenUrl` was
                // not compared before; it is now, for the same reason.
                match &rest.auth {
                    RestAuth::Oauth2ClientCredentials { token_url } => {
                        parts.push(("tokenUrl", origin_key(token_url)));
                    }
                    RestAuth::Oauth2AuthorizationCode {
                        token_url,
                        revocation_url,
                        ..
                    } => {
                        parts.push(("tokenUrl", origin_key(token_url)));
                        parts.push((
                            "revocationUrl",
                            revocation_url
                                .as_deref()
                                .map(origin_key)
                                .unwrap_or_default(),
                        ));
                    }
                    RestAuth::ApiKey { .. } | RestAuth::Bearer | RestAuth::Basic => {}
                }
                TargetIdentity::new("rest", parts)
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
            Dial::Saas(saas) => saas_target_identity(saas),
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

/// The target identity of a `saas` dial. `SRC-3` AD2: every address of a
/// `saas` connector is built from the vendor row and `account`, so these two
/// decide where the credential goes; the sign-in kind decides which secret
/// occupies which slot. Changing any of the three needs every credential again
/// (`SEC-14` part A; final review SHOULD-FIX 25: with the kind left out,
/// switching sign-in kept the stored refs in their slots and sent a secret in
/// another role). The vendor and the host part of the account compare
/// case-insensitively, as host names do elsewhere in this module; the tenant
/// after a `/` (Workday) is a name and compares as written.
fn saas_target_identity(saas: &SaasDial) -> TargetIdentity {
    let account = saas.account.trim();
    let account = match account.split_once('/') {
        Some((host, tenant)) => format!("{}/{tenant}", host.to_ascii_lowercase()),
        None => account.to_ascii_lowercase(),
    };
    let auth_kind = match &saas.auth {
        SaasAuth::Oauth2AuthorizationCode { .. } => "oauth2_authorization_code",
        SaasAuth::Integration => "integration",
    };
    TargetIdentity::new(
        "saas",
        vec![
            ("vendor", saas.vendor.trim().to_ascii_lowercase()),
            ("account", account),
            ("authType", auth_kind.to_owned()),
        ],
    )
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

/// The per-driver rule for a `cdc` dial's `slotName`, `publicationName` and
/// `serverId` (`SRC-4` decision S3-a). Every refusal is one fixed sentence
/// naming the field and the driver, never a value.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] when a field the driver needs is
/// missing, a field it does not use is present, or `serverId` is 0.
fn validate_cdc_dial_post_parse(adapter: &str, dial: &CdcDial) -> Result<(), IngestSpecError> {
    let refuse = |message: &'static str| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source: serde::de::Error::custom(message),
    };
    match dial.driver {
        SqlDriver::Postgres => {
            if dial.slot_name.is_none() {
                return Err(refuse(
                    "slotName is required for a PostgreSQL change-capture connector",
                ));
            }
            if dial.publication_name.is_none() {
                return Err(refuse(
                    "publicationName is required for a PostgreSQL change-capture connector",
                ));
            }
            if dial.server_id.is_some() {
                return Err(refuse(
                    "serverId is only for a MySQL change-capture connector",
                ));
            }
        }
        SqlDriver::Mysql => {
            match dial.server_id {
                None => {
                    return Err(refuse(
                        "serverId is required for a MySQL change-capture connector",
                    ));
                }
                Some(0) => return Err(refuse("serverId must be between 1 and 4294967295")),
                Some(_) => {}
            }
            if dial.slot_name.is_some() {
                return Err(refuse(
                    "slotName is only for a PostgreSQL change-capture connector",
                ));
            }
            if dial.publication_name.is_some() {
                return Err(refuse(
                    "publicationName is only for a PostgreSQL change-capture connector",
                ));
            }
        }
        SqlDriver::Mssql => {
            if dial.slot_name.is_some() {
                return Err(refuse(
                    "slotName is only for a PostgreSQL change-capture connector",
                ));
            }
            if dial.publication_name.is_some() {
                return Err(refuse(
                    "publicationName is only for a PostgreSQL change-capture connector",
                ));
            }
            if dial.server_id.is_some() {
                return Err(refuse(
                    "serverId is only for a MySQL change-capture connector",
                ));
            }
        }
        // Oracle change capture is slice 6; until then its dial is parsed as
        // before, with none of the three.
        SqlDriver::Oracle => {}
    }
    Ok(())
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

/// The longest `OAuth2` address, client id or scope accepted.
const MAX_OAUTH_FIELD_LEN: usize = 2048;
/// The most scopes a dial may ask for.
const MAX_OAUTH_SCOPES: usize = 50;

/// Whether `value` is an `http(s)://` address with a host, no user name or
/// password and no whitespace or control character. The scheme is not
/// narrowed to `https` here: the store does not know the installation's
/// settings. The API refuses `http` unless the operator's gate-only switch
/// allows it (`routes::connectors::check_dial_ssrf`), and the orchestrator
/// applies the same rule when it dials.
fn is_plain_http_address(value: &str) -> bool {
    let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    else {
        return false;
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    !authority.is_empty()
        && !authority.contains('@')
        && !value.chars().any(|c| c.is_control() || c.is_whitespace())
        && value.len() <= MAX_OAUTH_FIELD_LEN
}

/// Post-deserialize validation for a [`RestDial`]: the `OAuth2` addresses and
/// names serde cannot check (`SRC-12` part C).
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] for an `OAuth2` address that is
/// not a plain `http(s)` address, an empty or oversized client id, or a scope
/// that is not a single token.
fn validate_rest_dial_post_parse(adapter: &str, dial: &RestDial) -> Result<(), IngestSpecError> {
    let refuse = |message: &str| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source: serde::de::Error::custom(message.to_owned()),
    };
    match &dial.auth {
        RestAuth::Oauth2ClientCredentials { token_url } => {
            if !is_plain_http_address(token_url) {
                return Err(refuse(
                    "auth.tokenUrl must be an http(s):// address with a host and no user name or password",
                ));
            }
        }
        RestAuth::Oauth2AuthorizationCode {
            client_id,
            authorize_url,
            token_url,
            revocation_url,
            scopes,
        } => {
            if client_id.trim().is_empty()
                || client_id.len() > MAX_OAUTH_FIELD_LEN
                || client_id.chars().any(char::is_control)
            {
                return Err(refuse("auth.clientId must be a non-empty name"));
            }
            for (field, value) in [
                ("auth.authorizeUrl", Some(authorize_url)),
                ("auth.tokenUrl", Some(token_url)),
                ("auth.revocationUrl", revocation_url.as_ref()),
            ] {
                if value.is_some_and(|v| !is_plain_http_address(v)) {
                    return Err(refuse(&format!(
                        "{field} must be an http(s):// address with a host and no user name or password"
                    )));
                }
            }
            if scopes.len() > MAX_OAUTH_SCOPES
                || scopes.iter().any(|scope| {
                    scope.is_empty()
                        || scope.len() > MAX_OAUTH_FIELD_LEN
                        || scope
                            .chars()
                            .any(|c| c.is_control() || c.is_whitespace() || c == '"')
                })
            {
                return Err(refuse(
                    "auth.scopes must be a short list of single-word scopes",
                ));
            }
        }
        RestAuth::ApiKey { .. } | RestAuth::Bearer | RestAuth::Basic => {}
    }
    Ok(())
}

/// The most objects a `saas` dial may name.
const MAX_SAAS_OBJECTS: usize = 200;
/// The longest object name a `saas` dial may carry.
const MAX_SAAS_OBJECT_NAME_LEN: usize = 256;

/// Post-deserialize validation for a [`SaasDial`] (`SRC-3` AD1, AD2): the
/// vendor is in the table, the account has the shape the vendor's host
/// pattern allows, the sign-in is one the vendor offers, and the object
/// names are plain text.
///
/// # Errors
///
/// Returns [`IngestSpecError::InvalidDial`] for an unknown vendor, an account
/// outside the vendor's pattern, an integration credential the vendor does
/// not offer (Salesforce: decision 4) or `NetSuite`'s without its
/// `certificateId` (final review SHOULD-FIX 25), an empty or oversized
/// client id, or an object name that is blank, long, repeated or has a
/// control character.
fn validate_saas_dial_post_parse(adapter: &str, dial: &SaasDial) -> Result<(), IngestSpecError> {
    let refuse = |message: &str| IngestSpecError::InvalidDial {
        adapter: adapter.to_owned(),
        source: serde::de::Error::custom(message.to_owned()),
    };
    let Some(vendor) = crate::saas_vendors::vendor(&dial.vendor) else {
        return Err(refuse("vendor is not one this build offers"));
    };
    vendor
        .validate_account(&dial.account)
        .map_err(|err| refuse(&format!("account: {err}")))?;
    match &dial.auth {
        SaasAuth::Oauth2AuthorizationCode { client_id } => {
            if client_id.trim().is_empty()
                || client_id.len() > MAX_OAUTH_FIELD_LEN
                || client_id.chars().any(char::is_control)
            {
                return Err(refuse("auth.clientId must be a non-empty name"));
            }
        }
        SaasAuth::Integration => {
            if vendor.integration.is_none() {
                return Err(refuse(
                    "this vendor does not offer an integration credential; sign in instead",
                ));
            }
        }
    }
    if let Some(certificate_id) = &dial.certificate_id {
        // Slice 6: only `NetSuite`'s client-credentials way has a certificate; the id goes into a
        // JWT header, so it is a short plain name.
        let plain = !certificate_id.is_empty()
            && certificate_id.len() <= 128
            && certificate_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
        if dial.vendor != "netsuite" || dial.auth != SaasAuth::Integration || !plain {
            return Err(refuse(
                "certificateId is only for NetSuite's integration credential and must be a plain name",
            ));
        }
    } else if dial.vendor == "netsuite" && dial.auth == SaasAuth::Integration {
        // PR #216 final review SHOULD-FIX 25: without it the signed assertion cannot name the
        // certificate uploaded in NetSuite, and the connector failed only at its connection test.
        return Err(refuse(
            "NetSuite's integration credential needs certificateId: the id of the certificate uploaded in NetSuite",
        ));
    }
    let mut seen = std::collections::HashSet::new();
    if dial.objects.len() > MAX_SAAS_OBJECTS
        || dial.objects.iter().any(|name| {
            name.trim().is_empty()
                || name.len() > MAX_SAAS_OBJECT_NAME_LEN
                || name.chars().any(char::is_control)
                || !seen.insert(name.as_str())
        })
    {
        return Err(refuse(
            "objects must be a short list of distinct, non-blank names without control characters",
        ));
    }
    Ok(())
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
        // SRC-4 slice 1 review SHOULD-FIX 3: the Oracle `database` is a service name, and a
        // service name is an identifier. Letters, digits, `_`, `.` and `-` (the characters a
        // domain-qualified service name such as `FREEPDB1` or `sales.example.com` uses) in
        // 1 to 128 characters; the same rule is in `adapters/oracle.py`. The sentence is fixed
        // and does not repeat the value.
        if !is_oracle_service_name(&dial.database) {
            return Err(IngestSpecError::InvalidDial {
                adapter: adapter.to_owned(),
                source: serde::de::Error::custom(
                    "database must be an Oracle service name: 1 to 128 letters, digits, \
                     underscores, dots or hyphens",
                ),
            });
        }
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

/// Whether `name` is an identifier fit to be an Oracle service name
/// (`^[A-Za-z0-9_.-]{1,128}$`; SRC-4 slice 1 review SHOULD-FIX 3).
fn is_oracle_service_name(name: &str) -> bool {
    (1..=128).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
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
    /// `sql | cdc | mongodb | kafka | sftp | files | rest | sheets | saas` set.
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
/// what acts on it; the wire form is the snake-case name (`SRC-10`: the two
/// deduplicating modes are `replace_dedup` and `incremental_dedup`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoadMode {
    /// Overwrite the table with what the source returns now.
    Replace,
    /// Add every row the source returns; two runs leave two copies.
    Append,
    /// Add only rows beyond the cursor column's last seen value.
    Incremental,
    /// Overwrite the table with what the source returns now, keeping one
    /// row per [`SourceObject::primary_key`] (`SRC-10`).
    ReplaceDedup,
    /// Merge the rows beyond the cursor column's last seen value into the
    /// table on [`SourceObject::primary_key`] (`SRC-10`).
    IncrementalDedup,
    /// Keep every version of a row: a changed row is closed and a new
    /// version added, each with the time of the load that saw it
    /// (`SRC-10` decisions 1, 9 and 10). Reads the whole source on every
    /// run, so it takes a key and no cursor.
    History,
}

impl LoadMode {
    /// Whether the mode keeps one row per key, and so needs a key.
    #[must_use]
    pub const fn is_dedup(self) -> bool {
        matches!(self, Self::ReplaceDedup | Self::IncrementalDedup)
    }

    /// Whether the mode identifies rows by [`SourceObject::primary_key`]: the
    /// deduplicating modes and `history`. A column of that key removed at the
    /// source breaks the table under every one of them (`SRC-10` decision 6).
    #[must_use]
    pub const fn needs_key(self) -> bool {
        self.is_dedup() || matches!(self, Self::History)
    }

    /// Whether the mode reads only rows beyond a cursor, and so needs a
    /// cursor column and an adapter that can filter by one.
    #[must_use]
    pub const fn is_incremental(self) -> bool {
        matches!(self, Self::Incremental | Self::IncrementalDedup)
    }
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
    /// The columns that identify one row, for the deduplicating modes
    /// (`SRC-10`). Kept on the object whatever the mode, so the console
    /// can hold on to a detected key while the user switches modes.
    #[serde(default)]
    pub primary_key: Option<Vec<String>>,
    /// The columns of the source table the load leaves out (`SRC-11`
    /// decisions D9 to D12). Stored as what is left OUT, not what is kept, so
    /// a column that appears at the source later follows the connector's
    /// schema-change policy instead of being left out silently. Absent or
    /// empty: nothing is left out. See [`validate_excluded_columns`].
    #[serde(default)]
    pub excluded_columns: Option<Vec<String>>,
    /// The Bronze target this object lands at.
    pub target: String,
}

/// The longest cursor column name accepted, generous for any SQL
/// identifier (`PostgreSQL` 63, `MySQL` 64, SQL Server 128).
const MAX_INCREMENTAL_KEY_LEN: usize = 128;

/// The most columns a `primaryKey` may name (`SRC-10`).
const MAX_PRIMARY_KEY_COLUMNS: usize = 16;

/// The longest column name in a `primaryKey`, as for the cursor column.
const MAX_PRIMARY_KEY_COLUMN_LEN: usize = 128;

/// The columns a `history` table adds (`SRC-10` decision 1). Mirrors
/// `HISTORY_COLUMNS` in `dagster/dispar_orchestrate/adapters/history.py`.
const HISTORY_COLUMNS: [&str; 4] = ["_valid_from", "_valid_to", "_is_current", "_row_hash"];

/// Why a spec's `sourceObjects` cannot be ingested as written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidLoadMode {
    /// `loadMode` is not a [`LoadMode`].
    #[error(
        "source object {name:?} has loadMode {mode:?}; use replace, append, incremental, \
         replace_dedup, incremental_dedup or history"
    )]
    Unknown {
        /// The source object's name.
        name: String,
        /// The offending value, as sent.
        mode: String,
    },
    /// `incremental` or `incremental_dedup` for an adapter the ingest job
    /// does not run it for.
    #[error(
        "source object {name:?} asks for an incremental loadMode, which only SQL and business-app \
         connectors support"
    )]
    NotForAdapter {
        /// The source object's name.
        name: String,
    },
    /// `incremental` or `incremental_dedup` without a usable cursor column.
    #[error(
        "source object {name:?} asks for an incremental loadMode, which needs incrementalKey: the \
         column that marks new rows"
    )]
    NoCursor {
        /// The source object's name.
        name: String,
    },
    /// Plain `incremental` on a `saas` object (final review BLOCKER 18): the
    /// mode appends, so a vendor record read again after a change becomes a
    /// second row under the same vendor id. Only a mode that merges on a key
    /// keeps the table one row per record (`SRC-3` decision 6).
    #[error(
        "source object {name:?} asks for loadMode incremental on a business-app connector, which \
         adds a changed record as a second row; use incremental_dedup, which keeps one row per key"
    )]
    SaasNeedsMerge {
        /// The source object's name.
        name: String,
    },
    /// A load mode a change-capture connector does not run (`SRC-4` decision
    /// S2-g): it keeps the current row per key (`incremental_dedup`, the
    /// default) or every version (`history`), nothing else.
    #[error(
        "source object {name:?} has a loadMode that change capture does not run; use \
         incremental_dedup (the current row per key) or history (every version)"
    )]
    NotForChangeCapture {
        /// The source object's name.
        name: String,
    },
    /// `replace_dedup` or `incremental_dedup` without a key (`SRC-10`).
    #[error(
        "source object {name:?} asks for a deduplicating loadMode, which needs primaryKey: the \
         columns that identify one row"
    )]
    NoKey {
        /// The source object's name.
        name: String,
    },
    /// `history` with an `incrementalKey` (`SRC-10` decision 10): this version
    /// reads the whole source on every run.
    #[error(
        "source object {name:?} asks for loadMode history, which reads the whole source on every \
         run and takes no incrementalKey"
    )]
    HistoryHasCursor {
        /// The source object's name.
        name: String,
    },
    /// `history` with a key column named like one of the columns the mode adds
    /// to the table (`SRC-10` decision 1).
    #[error(
        "source object {name:?} asks for loadMode history, whose primaryKey cannot name _valid_from, \
         _valid_to, _is_current or _row_hash: the mode adds those columns"
    )]
    ReservedKeyColumn {
        /// The source object's name.
        name: String,
    },
    /// A `primaryKey` that is not a short list of plain, distinct column
    /// names (`SRC-10`). Checked whatever the mode.
    #[error(
        "source object {name:?} has a primaryKey that is not a list of at most 16 distinct, \
         non-blank column names of at most 128 characters without control characters"
    )]
    BadKey {
        /// The source object's name.
        name: String,
    },
}

/// Whether `columns` is a key the loader can use: 1 to
/// [`MAX_PRIMARY_KEY_COLUMNS`] distinct names, none blank, none longer than
/// [`MAX_PRIMARY_KEY_COLUMN_LEN`], none with a control character. A name is
/// compared and sent as written, so no trimming hides a difference between
/// two columns.
fn key_is_usable(columns: &[serde_json::Value]) -> bool {
    if columns.is_empty() || columns.len() > MAX_PRIMARY_KEY_COLUMNS {
        return false;
    }
    let mut seen = std::collections::HashSet::new();
    columns.iter().all(|column| {
        column.as_str().is_some_and(|name| {
            !name.trim().is_empty()
                && name.len() <= MAX_PRIMARY_KEY_COLUMN_LEN
                && !name.chars().any(char::is_control)
                && seen.insert(name)
        })
    })
}

/// Check each source object's load mode, the one part of `sourceObjects`
/// checked at save time: a mode the ingest job cannot run would otherwise
/// surface only as a rejected run. Everything else in an object is left
/// as it is, as before.
///
/// # Errors
///
/// Returns [`InvalidLoadMode`] for the first object with an unknown
/// `loadMode`, with an incremental mode on an adapter other than `sql` and `saas` or without a
/// cursor column that is a plain name of sensible length, with plain `incremental` on a `saas`
/// object (it appends; a business-app record must merge on its key, final review BLOCKER 18),
/// with a deduplicating mode and no `primaryKey`, or with a `primaryKey` that is
/// not a usable key. For `cdc` (`SRC-4` decision S2-g) only `incremental_dedup` (the
/// default when no mode is given) and `history` are accepted, both with a key, and
/// no cursor is asked for: [`InvalidLoadMode::NotForChangeCapture`] otherwise.
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
        // SRC-10: a malformed `primaryKey` is refused whatever the mode, so a
        // bad key cannot sit in a spec until the user switches to a mode that
        // reads it. A well-formed one on a mode that does not use it is
        // accepted and ignored: the console keeps the detected key on the
        // table while the user switches modes.
        let has_key = match object.get("primaryKey") {
            None | Some(serde_json::Value::Null) => false,
            Some(serde_json::Value::Array(columns)) if key_is_usable(columns) => true,
            Some(_) => return Err(InvalidLoadMode::BadKey { name }),
        };
        // `SRC-4` decision S2-g: a change-capture object with no mode is the
        // default, the current row per key, and so needs a key like any other.
        let change_capture = adapter == "cdc";
        let mode = match object.get("loadMode") {
            None | Some(serde_json::Value::Null) if change_capture => LoadMode::IncrementalDedup,
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
        if change_capture && !matches!(mode, LoadMode::IncrementalDedup | LoadMode::History) {
            return Err(InvalidLoadMode::NotForChangeCapture { name });
        }
        if mode.needs_key() && !has_key {
            return Err(InvalidLoadMode::NoKey { name });
        }
        if mode == LoadMode::History {
            // `SRC-10` decision 10: no cursor in this version. A blank cursor
            // is what the console sends when the field is empty, so only a
            // value is refused.
            let cursor = object
                .get("incrementalKey")
                .and_then(serde_json::Value::as_str)
                .map_or("", str::trim);
            if !cursor.is_empty() {
                return Err(InvalidLoadMode::HistoryHasCursor { name });
            }
            let names_reserved = object
                .get("primaryKey")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .any(|column| HISTORY_COLUMNS.contains(&column));
            if names_reserved {
                return Err(InvalidLoadMode::ReservedKeyColumn { name });
            }
        }
        // A change-capture `incremental_dedup` reads no cursor: the changes
        // arrive in order, and the key merges them.
        if !mode.is_incremental() || change_capture {
            continue;
        }
        // PR #216 final review BLOCKER 18: `incremental` appends, so every changed vendor record
        // becomes a second row under the same id (and with a lag dlt's boundary de-duplication is
        // off, so unchanged rows repeat too). A business-app object may only grow incrementally by
        // merging on its key, which is `incremental_dedup`.
        if adapter == "saas" && mode == LoadMode::Incremental {
            return Err(InvalidLoadMode::SaasNeedsMerge { name });
        }
        // `SRC-3` AD6: a `saas` object may load incrementally too. Whether the
        // vendor's API can filter that object by the cursor is declared per
        // object in the orchestrator's vendor module (`saas/objects.py`); an
        // object without a cursor there is saved without `incrementalKey`
        // and so still stops at `NoCursor` below.
        if !matches!(adapter, "sql" | "saas") {
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

/// The most columns an object may leave out (`SRC-11`); generous for any
/// table.
const MAX_EXCLUDED_COLUMNS: usize = 2000;

/// Why a source object's `excludedColumns` cannot be saved (`SRC-11`
/// decisions D9 and D11). The route answers [`Self::BadList`] with 400 and the
/// others with 422; every text is fixed, and names only the object and a
/// column the request itself named.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum InvalidExcludedColumns {
    /// Not a list of at most 2,000 distinct, non-blank column names of at
    /// most 128 characters without control characters.
    #[error(
        "source object {name:?} has an excludedColumns that is not a list of distinct, \
         non-blank column names of at most 128 characters without control characters"
    )]
    BadList {
        /// The source object's name.
        name: String,
    },
    /// The source cannot list its columns before loading (`SRC-11` decision
    /// D9): only `PostgreSQL`, `MySQL`, `MariaDB` and SQL Server can.
    #[error(
        "source object {name:?} leaves columns out, which is not available for this source type"
    )]
    NotForSource {
        /// The source object's name.
        name: String,
    },
    /// A key column of a deduplicating or history table (`SRC-11` decision
    /// D11): the table cannot identify its rows without it.
    #[error("source object {name:?} cannot leave out {column:?}: it is part of the table's key")]
    NamesKeyColumn {
        /// The source object's name.
        name: String,
        /// The key column that was named.
        column: String,
    },
    /// The cursor column of an incremental table (`SRC-11` decision D11): the
    /// load cannot find new rows without it.
    #[error("source object {name:?} cannot leave out {column:?}: it is the cursor column")]
    NamesCursorColumn {
        /// The source object's name.
        name: String,
        /// The cursor column that was named.
        column: String,
    },
}

/// Whether the dial's source can list its columns before loading, so that
/// column choice is offered for it (`SRC-11` decision D9): the `sql` adapter
/// with the `PostgreSQL`, `MySQL`/`MariaDB` or SQL Server driver, and the
/// `saas` adapter (`SRC-3` AD6: each vendor module lists an object's fields
/// before the load). Oracle follows with `SRC-13`; change-capture and every
/// other adapter are out.
#[must_use]
pub const fn supports_column_choice(dial: &Dial) -> bool {
    matches!(
        dial,
        Dial::Sql(SqlDial {
            driver: SqlDriver::Postgres | SqlDriver::Mysql | SqlDriver::Mssql | SqlDriver::Oracle,
            ..
        }) | Dial::Saas(_)
    )
}

/// `object`'s `excludedColumns` as names, `Ok(None)` when absent, null or
/// empty (nothing left out).
fn excluded_names(
    object: &serde_json::Value,
    name: &str,
) -> Result<Option<Vec<String>>, InvalidExcludedColumns> {
    let bad = || InvalidExcludedColumns::BadList {
        name: name.to_owned(),
    };
    let list = match object.get("excludedColumns") {
        None | Some(serde_json::Value::Null) => return Ok(None),
        Some(serde_json::Value::Array(list)) => list,
        Some(_) => return Err(bad()),
    };
    if list.len() > MAX_EXCLUDED_COLUMNS {
        return Err(bad());
    }
    let mut seen = std::collections::HashSet::new();
    let mut names = Vec::with_capacity(list.len());
    for column in list {
        let Some(column) = column.as_str() else {
            return Err(bad());
        };
        if column.trim().is_empty()
            || column.len() > MAX_PRIMARY_KEY_COLUMN_LEN
            || column.chars().any(char::is_control)
            || !seen.insert(column)
        {
            return Err(bad());
        }
        names.push(column.to_owned());
    }
    Ok((!names.is_empty()).then_some(names))
}

/// Check each source object's `excludedColumns` at save time (`SRC-11`
/// decisions D9 to D11): well-formed, only for a source that can list its
/// columns ([`supports_column_choice`]), and never naming a key column of a
/// deduplicating or history table or the cursor column of an incremental one.
///
/// What this cannot check is that a named column exists: the columns are
/// read from the source at load time, and a name the source does not have
/// simply leaves nothing out.
///
/// # Errors
///
/// Returns [`InvalidExcludedColumns`] for the first object that breaks a rule.
pub fn validate_excluded_columns(
    dial: &Dial,
    source_objects: &serde_json::Value,
) -> Result<(), InvalidExcludedColumns> {
    for object in source_objects.as_array().into_iter().flatten() {
        let name = object
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let Some(excluded) = excluded_names(object, name)? else {
            continue;
        };
        if !supports_column_choice(dial) {
            return Err(InvalidExcludedColumns::NotForSource {
                name: name.to_owned(),
            });
        }
        let mode = object
            .get("loadMode")
            .and_then(|value| LoadMode::deserialize(value).ok());
        if mode.is_some_and(LoadMode::needs_key) {
            let key = object
                .get("primaryKey")
                .and_then(serde_json::Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str);
            for column in key {
                if excluded.iter().any(|e| e == column) {
                    return Err(InvalidExcludedColumns::NamesKeyColumn {
                        name: name.to_owned(),
                        column: column.to_owned(),
                    });
                }
            }
        }
        if mode.is_some_and(LoadMode::is_incremental) {
            let cursor = object
                .get("incrementalKey")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if excluded.iter().any(|e| e == cursor) {
                return Err(InvalidExcludedColumns::NamesCursorColumn {
                    name: name.to_owned(),
                    column: cursor.to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// The `(object, column)` pairs that are to be marked inactive: the columns
/// left out of a table that KEEPS its rows (`append`, `incremental`,
/// `incremental_dedup`, `history`), where a left-out column stays in the
/// Bronze table with no new values (`SRC-11` decision D10). In a table the
/// next load replaces (`replace`, `replace_dedup`, or no mode) the column is
/// gone after the load and nothing is marked. Lenient like [`dedup_keys`]:
/// a stored spec has passed [`validate_excluded_columns`].
#[must_use]
pub fn inactive_exclusions(source_objects: &serde_json::Value) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    for object in source_objects.as_array().into_iter().flatten() {
        let Some(name) = object.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let keeps_rows = object
            .get("loadMode")
            .and_then(|value| LoadMode::deserialize(value).ok())
            .is_some_and(|mode| !matches!(mode, LoadMode::Replace | LoadMode::ReplaceDedup));
        if !keeps_rows {
            continue;
        }
        let Ok(Some(excluded)) = excluded_names(object, name) else {
            continue;
        };
        pairs.extend(excluded.into_iter().map(|column| (name.to_owned(), column)));
    }
    pairs
}

/// The key columns of every object whose stored load mode has a key (the
/// deduplicating modes and `history`, [`LoadMode::needs_key`]), by
/// object name (`SRC-10` decision 6). Read by the schema-change code from the
/// STORED spec, never from what the orchestrator reports, so a request cannot
/// claim a table has no key. Lenient: an object [`validate_load_modes`] would
/// refuse cannot be in a stored spec, and a malformed one simply has no entry
/// (treated as non-deduplicating, as before `SRC-10`).
#[must_use]
pub fn dedup_keys(
    source_objects: &serde_json::Value,
) -> std::collections::HashMap<String, Vec<String>> {
    let mut keys = std::collections::HashMap::new();
    for object in source_objects.as_array().into_iter().flatten() {
        let Some(name) = object.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let is_dedup = object
            .get("loadMode")
            .and_then(|value| LoadMode::deserialize(value).ok())
            .is_some_and(LoadMode::needs_key);
        if !is_dedup {
            continue;
        }
        let key: Vec<String> = object
            .get("primaryKey")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|column| column.as_str().map(str::to_owned))
            .collect();
        keys.insert(name.to_owned(), key);
    }
    keys
}

/// `text` as a lower-case SQL-safe slug: every run of characters outside
/// `[a-z0-9]` becomes one `_`, and leading and trailing `_` are dropped.
fn bronze_slug(text: &str) -> String {
    let mut out = String::new();
    let mut separator_pending = false;
    for c in text.to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if separator_pending && !out.is_empty() {
                out.push('_');
            }
            separator_pending = false;
            out.push(c);
        } else {
            separator_pending = true;
        }
    }
    out
}

/// The Bronze table a source object lands in unless a person renames it:
/// `<connector>_<table>` (`SRC-8` task 8, decision D2).
///
/// Every connector writes into the same flat `bronze` namespace, so two
/// connectors that both have an `orders` table would otherwise append into
/// one `bronze.orders`. `object_name` is `schema.table`; only its last
/// dot-separated part is used.
///
/// THE SAME RULE AS THE CONSOLE'S TABLE PICKER: this is a port of
/// `defaultTarget` in `src/features/connectors/connector-ingest-panel.tsx`.
/// Keep both in step; each has a test that pins the same three examples
/// (`northwind`/`public.orders`, `Northwind DB!`/`public.Order Details`,
/// `2024 sales`/`t`).
#[must_use]
pub fn default_bronze_target(connector_name: &str, object_name: &str) -> String {
    let table = bronze_slug(object_name.rsplit('.').next().unwrap_or(object_name));
    let prefix = bronze_slug(connector_name);
    let joined = [prefix, table]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join("_");
    let joined = if joined.is_empty() {
        "table".to_owned()
    } else {
        joined
    };
    if joined.starts_with(|c: char| c.is_ascii_digit()) {
        format!("t_{joined}")
    } else {
        joined
    }
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
/// `auth_type` is consulted for `adapter == "saas"` (the key
/// [`Dial::saas_auth_type`] builds from the vendor table), `adapter == "rest"`
/// (the `dial.auth.type` tag), `adapter == "kafka"` (`"none"` needs zero secret refs — a
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
        // `SRC-12` part C: the client id is configuration in the dial, so the
        // two secrets are the client secret (primary, typed or referenced)
        // and the refresh token (secondary, written by the sign-in callback).
        ("rest", Some("oauth2_authorization_code")) => Some(&["clientSecret", "refreshToken"]),
        ("kafka", Some("none")) => Some(&[]),
        ("sftp", Some("public_key")) => Some(&["privateKey"]),
        ("sheets", _) => Some(&["serviceAccountJson"]),
        // `SRC-3` AD2/AD8: a `saas` connector's slots come from the vendor
        // table, keyed by `Dial::saas_auth_type` (the browser sign-in's two
        // secrets, or the vendor's integration credential's fields).
        ("saas", Some(auth_type)) => crate::saas_vendors::secret_fields_for(auth_type),
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

    /// The same examples `connector-ingest-panel.test.tsx` pins for
    /// `defaultTarget` (the console's table picker): the two must agree.
    #[test]
    fn the_default_bronze_target_matches_the_console_pickers_examples() {
        assert_eq!(
            default_bronze_target("northwind", "public.orders"),
            "northwind_orders"
        );
        assert_eq!(
            default_bronze_target("Northwind DB!", "public.Order Details"),
            "northwind_db_order_details"
        );
        assert_eq!(default_bronze_target("2024 sales", "t"), "t_2024_sales_t");
    }

    #[test]
    fn the_default_bronze_target_falls_back_the_way_the_picker_does() {
        // No usable connector name: the table alone.
        assert_eq!(default_bronze_target("!!!", "dbo.Orders"), "orders");
        // No usable table part: the connector alone.
        assert_eq!(default_bronze_target("shop", "public."), "shop");
        // Nothing usable at all.
        assert_eq!(default_bronze_target("", "..."), "table");
        // A name without a schema is its own table part.
        assert_eq!(default_bronze_target("shop", "orders"), "shop_orders");
        // Runs of separators collapse and edges are trimmed.
        assert_eq!(default_bronze_target("  a--b  ", "s.__x__y__"), "a_b_x_y");
    }

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
    fn cdc_dial_fields_follow_the_driver() {
        let base = |driver: &str| {
            serde_json::json!({
                "driver": driver, "host": "db.internal", "port": 5432,
                "database": "orders", "user": "app_reader"
            })
        };
        let refusal = |json: serde_json::Value| Dial::parse("cdc", &json).unwrap_err().to_string();
        // PostgreSQL: both names required, no server id; stored rows parse unchanged.
        assert!(refusal(base("postgres")).contains("slotName is required"));
        let mut pg = base("postgres");
        pg["slotName"] = serde_json::json!("s");
        assert!(refusal(pg.clone()).contains("publicationName is required"));
        pg["publicationName"] = serde_json::json!("p");
        assert!(Dial::parse("cdc", &pg).is_ok());
        pg["serverId"] = serde_json::json!(5);
        assert!(refusal(pg).contains("serverId is only for a MySQL"));
        // MySQL: a server id in 1..=4294967295, neither name.
        assert!(refusal(base("mysql")).contains("serverId is required"));
        let mut my = base("mysql");
        my["serverId"] = serde_json::json!(0);
        assert!(refusal(my.clone()).contains("between 1 and 4294967295"));
        my["serverId"] = serde_json::json!(4_294_967_295_u64);
        assert!(Dial::parse("cdc", &my).is_ok());
        my["serverId"] = serde_json::json!(4_294_967_296_u64);
        assert!(Dial::parse("cdc", &my).is_err());
        my["serverId"] = serde_json::json!(7);
        my["slotName"] = serde_json::json!("s");
        assert!(refusal(my.clone()).contains("slotName is only for a PostgreSQL"));
        my.as_object_mut().unwrap().remove("slotName");
        my["publicationName"] = serde_json::json!("p");
        assert!(refusal(my).contains("publicationName is only for a PostgreSQL"));
        // SQL Server: none of the three.
        assert!(Dial::parse("cdc", &base("mssql")).is_ok());
        for (field, value) in [
            ("slotName", serde_json::json!("s")),
            ("publicationName", serde_json::json!("p")),
            ("serverId", serde_json::json!(1)),
        ] {
            let mut ms = base("mssql");
            ms[field] = value;
            assert!(
                Dial::parse("cdc", &ms).is_err(),
                "{field} must be refused for SQL Server"
            );
        }
    }

    #[test]
    fn cdc_dial_ssl_mode_defaults_to_require_and_names_a_closed_set() {
        let mut json = serde_json::json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "app_reader",
            "slotName": "orders_slot", "publicationName": "orders_pub"
        });
        let dial = serde_json::from_value::<CdcDial>(json.clone()).unwrap();
        assert_eq!(dial.ssl_mode, CdcSslMode::Require);
        for (word, mode) in [
            ("disable", CdcSslMode::Disable),
            ("require", CdcSslMode::Require),
            ("verify-ca", CdcSslMode::VerifyCa),
            ("verify-full", CdcSslMode::VerifyFull),
        ] {
            json["sslMode"] = serde_json::json!(word);
            let dial = serde_json::from_value::<CdcDial>(json.clone()).unwrap();
            assert_eq!(dial.ssl_mode, mode);
            assert_eq!(dial.ssl_mode.as_debezium(), word);
        }
        json["sslMode"] = serde_json::json!("prefer");
        assert!(serde_json::from_value::<CdcDial>(json).is_err());
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
            dial.as_cdc().and_then(|cdc| cdc.slot_name.as_deref()),
            Some("orders_slot")
        );
    }

    #[test]
    fn source_object_validate_rejects_an_empty_target() {
        let object = SourceObject {
            name: "public.orders".to_owned(),
            incremental_key: None,
            load_mode: None,
            primary_key: None,
            excluded_columns: None,
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
            primary_key: None,
            excluded_columns: None,
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

    /// `SRC-10` decision 6: only objects in a deduplicating mode have a key
    /// entry; a key kept on a table in another mode is not one.
    #[test]
    fn dedup_keys_lists_only_the_deduplicating_objects() {
        let keys = dedup_keys(&serde_json::json!([
            {"name": "a", "target": "a", "loadMode": "replace_dedup", "primaryKey": ["id"]},
            {"name": "b", "target": "b", "loadMode": "incremental_dedup", "primaryKey": ["x", "y"],
             "incrementalKey": "u"},
            {"name": "c", "target": "c", "loadMode": "replace", "primaryKey": ["id"]},
            {"name": "d", "target": "d", "primaryKey": ["id"]},
            {"target": "e", "loadMode": "replace_dedup", "primaryKey": ["id"]},
        ]));
        assert_eq!(keys.len(), 2);
        assert_eq!(keys["a"], vec!["id".to_owned()]);
        assert_eq!(keys["b"], vec!["x".to_owned(), "y".to_owned()]);
        assert!(dedup_keys(&serde_json::json!({})).is_empty());
    }

    #[test]
    fn source_object_reads_the_deduplicating_modes_and_a_primary_key() {
        let object: SourceObject = serde_json::from_value(serde_json::json!({
            "name": "public.orders", "target": "orders", "loadMode": "incremental_dedup",
            "incrementalKey": "updated_at", "primaryKey": ["id", "line"]
        }))
        .expect("valid");
        assert_eq!(object.load_mode, Some(LoadMode::IncrementalDedup));
        assert_eq!(
            object.primary_key,
            Some(vec!["id".to_owned(), "line".to_owned()])
        );
        let replace: SourceObject = serde_json::from_value(serde_json::json!({
            "name": "a", "target": "a", "loadMode": "replace_dedup", "primaryKey": ["id"]
        }))
        .expect("valid");
        assert_eq!(replace.load_mode, Some(LoadMode::ReplaceDedup));
        let none: SourceObject =
            serde_json::from_value(serde_json::json!({ "name": "a", "target": "a" }))
                .expect("valid");
        assert_eq!(none.primary_key, None);
    }

    #[test]
    fn validate_load_modes_accepts_a_deduplicating_mode_with_a_key() {
        let objects = serde_json::json!([
            { "name": "a", "target": "a", "loadMode": "replace_dedup", "primaryKey": ["id"] },
            { "name": "b", "target": "b", "loadMode": "incremental_dedup",
              "incrementalKey": "updated_at", "primaryKey": ["id", "line"] },
        ]);
        assert_eq!(validate_load_modes("sql", &objects), Ok(()));
        // `replace_dedup` runs on every batch adapter, `incremental_dedup` on `sql` only.
        let replace = serde_json::json!([
            { "name": "a", "target": "a", "loadMode": "replace_dedup", "primaryKey": ["id"] },
        ]);
        assert_eq!(validate_load_modes("files", &replace), Ok(()));
    }

    #[test]
    fn validate_load_modes_refuses_a_deduplicating_mode_without_a_key() {
        for mode in ["replace_dedup", "incremental_dedup"] {
            for object in [
                serde_json::json!({ "name": "a", "target": "a", "loadMode": mode, "incrementalKey": "u" }),
                serde_json::json!({ "name": "a", "target": "a", "loadMode": mode, "incrementalKey": "u", "primaryKey": [] }),
                serde_json::json!({ "name": "a", "target": "a", "loadMode": mode, "incrementalKey": "u", "primaryKey": null }),
            ] {
                assert!(
                    matches!(
                        validate_load_modes("sql", &serde_json::json!([object])),
                        Err(InvalidLoadMode::NoKey { .. } | InvalidLoadMode::BadKey { .. })
                    ),
                    "{mode} without a key must be refused"
                );
            }
        }
        assert!(matches!(
            validate_load_modes(
                "sql",
                &serde_json::json!([{ "name": "a", "target": "a", "loadMode": "replace_dedup" }])
            ),
            Err(InvalidLoadMode::NoKey { .. })
        ));
    }

    /// `SRC-4` decision S2-g: change capture runs the current row per key (the
    /// default) and history, both need a key, neither asks for a cursor.
    #[test]
    fn validate_load_modes_for_change_capture_accepts_two_modes_with_a_key() {
        let check =
            |object: serde_json::Value| validate_load_modes("cdc", &serde_json::json!([object]));
        for object in [
            serde_json::json!({ "name": "a", "target": "a", "primaryKey": ["id"] }),
            serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental_dedup",
                "primaryKey": ["id"] }),
            serde_json::json!({ "name": "a", "target": "a", "loadMode": "history",
                "primaryKey": ["id"] }),
        ] {
            assert_eq!(check(object.clone()), Ok(()), "{object}");
        }
        for mode in ["replace", "append", "incremental", "replace_dedup"] {
            assert!(
                matches!(
                    check(
                        serde_json::json!({ "name": "a", "target": "a", "loadMode": mode,
                        "incrementalKey": "u", "primaryKey": ["id"] })
                    ),
                    Err(InvalidLoadMode::NotForChangeCapture { .. })
                ),
                "{mode}"
            );
        }
        // No key, with or without a stated mode, is refused.
        for object in [
            serde_json::json!({ "name": "a", "target": "a" }),
            serde_json::json!({ "name": "a", "target": "a", "loadMode": "history" }),
            serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental_dedup" }),
        ] {
            assert!(
                matches!(check(object.clone()), Err(InvalidLoadMode::NoKey { .. })),
                "{object}"
            );
        }
        // History still takes no cursor.
        assert!(matches!(
            check(
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "history",
                "incrementalKey": "u", "primaryKey": ["id"] })
            ),
            Err(InvalidLoadMode::HistoryHasCursor { .. })
        ));
        // An empty list of objects (nothing chosen yet) is still saved.
        assert_eq!(validate_load_modes("cdc", &serde_json::json!([])), Ok(()));
    }

    #[test]
    fn validate_load_modes_requires_sql_and_a_cursor_for_incremental_dedup_as_for_incremental() {
        let check = |adapter: &str, object: serde_json::Value| {
            validate_load_modes(adapter, &serde_json::json!([object]))
        };
        assert!(matches!(
            check(
                "rest",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental_dedup",
                    "incrementalKey": "id", "primaryKey": ["id"] })
            ),
            Err(InvalidLoadMode::NotForAdapter { .. })
        ));
        assert!(matches!(
            check(
                "sql",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental_dedup",
                    "primaryKey": ["id"] })
            ),
            Err(InvalidLoadMode::NoCursor { .. })
        ));
        assert!(matches!(
            check(
                "sql",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "incremental_dedup",
                    "incrementalKey": " ", "primaryKey": ["id"] })
            ),
            Err(InvalidLoadMode::NoCursor { .. })
        ));
    }

    /// `SRC-10` sixth pass, decisions 1 and 10: `history` needs a key, runs on every
    /// batch adapter, takes no cursor and cannot key on a column it adds.
    #[test]
    fn validate_load_modes_for_history_needs_a_key_takes_no_cursor_and_keeps_its_own_columns_free()
    {
        let check = |adapter: &str, object: serde_json::Value| {
            validate_load_modes(adapter, &serde_json::json!([object]))
        };
        let history = |extra: serde_json::Value| {
            let mut object =
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "history" });
            for (field, value) in extra.as_object().into_iter().flatten() {
                object[field] = value.clone();
            }
            object
        };
        for adapter in ["sql", "files", "rest", "mongodb", "sftp"] {
            assert_eq!(
                check(
                    adapter,
                    history(serde_json::json!({ "primaryKey": ["id", "line"] }))
                ),
                Ok(()),
                "history on {adapter}"
            );
        }
        // A blank cursor is what an empty console field sends: not a cursor.
        assert_eq!(
            check(
                "sql",
                history(serde_json::json!({ "primaryKey": ["id"], "incrementalKey": "  " }))
            ),
            Ok(())
        );
        assert!(matches!(
            check("sql", history(serde_json::json!({}))),
            Err(InvalidLoadMode::NoKey { .. })
        ));
        assert!(matches!(
            check("sql", history(serde_json::json!({ "primaryKey": [] }))),
            Err(InvalidLoadMode::BadKey { .. })
        ));
        assert!(matches!(
            check(
                "sql",
                history(
                    serde_json::json!({ "primaryKey": ["id"], "incrementalKey": "updated_at" })
                )
            ),
            Err(InvalidLoadMode::HistoryHasCursor { .. })
        ));
        for reserved in ["_valid_from", "_valid_to", "_is_current", "_row_hash"] {
            assert!(
                matches!(
                    check(
                        "sql",
                        history(serde_json::json!({ "primaryKey": ["id", reserved] }))
                    ),
                    Err(InvalidLoadMode::ReservedKeyColumn { .. })
                ),
                "{reserved} in the key"
            );
        }
        // Another mode may keep such a name in its key: only `history` adds the columns.
        assert_eq!(
            check(
                "sql",
                serde_json::json!({ "name": "a", "target": "a", "loadMode": "replace_dedup",
                    "primaryKey": ["_valid_from"] })
            ),
            Ok(())
        );
    }

    /// `SRC-10` decision 6 for `history`: its key is read like a dedup table's, so a
    /// removed key column waits and cannot be approved (the same code path).
    #[test]
    fn dedup_keys_lists_a_history_object_with_its_key() {
        let keys = dedup_keys(&serde_json::json!([
            {"name": "h", "target": "h", "loadMode": "history", "primaryKey": ["id"]},
            {"name": "r", "target": "r", "loadMode": "replace", "primaryKey": ["id"]},
        ]));
        assert_eq!(keys.len(), 1);
        assert_eq!(keys.get("h"), Some(&vec!["id".to_owned()]));
        let object = SourceObject::deserialize(&serde_json::json!({
            "name": "h", "target": "h", "loadMode": "history", "primaryKey": ["id"]
        }))
        .expect("valid");
        assert_eq!(object.load_mode, Some(LoadMode::History));
        assert!(LoadMode::History.needs_key() && !LoadMode::History.is_dedup());
        assert!(!LoadMode::History.is_incremental());
    }

    #[test]
    fn validate_load_modes_refuses_a_primary_key_that_is_not_a_short_list_of_plain_distinct_names()
    {
        let too_many: Vec<String> = (0..17).map(|n| format!("c{n}")).collect();
        let sixteen: Vec<String> = (0..16).map(|n| format!("c{n}")).collect();
        let long = "x".repeat(129);
        let longest = "x".repeat(128);
        let bad = |key: serde_json::Value| {
            matches!(
                validate_load_modes(
                    "sql",
                    &serde_json::json!([{ "name": "a", "target": "a", "primaryKey": key }])
                ),
                Err(InvalidLoadMode::BadKey { .. })
            )
        };
        assert!(bad(serde_json::json!(too_many)), "17 columns");
        assert!(bad(serde_json::json!([])), "no column");
        assert!(bad(serde_json::json!([long])), "129 characters");
        assert!(bad(serde_json::json!([""])), "empty");
        assert!(bad(serde_json::json!(["  "])), "blank");
        assert!(bad(serde_json::json!(["a\nb"])), "control character");
        assert!(bad(serde_json::json!(["id", "id"])), "repeated");
        assert!(bad(serde_json::json!(["id", 7])), "not a string");
        assert!(bad(serde_json::json!("id")), "not a list");
        assert!(!bad(serde_json::json!(sixteen)), "16 columns are allowed");
        assert!(
            !bad(serde_json::json!([longest])),
            "128 characters are allowed"
        );
    }

    #[test]
    fn validate_load_modes_accepts_and_ignores_a_primary_key_on_a_mode_that_does_not_use_it() {
        // The console keeps the key it detected while the user switches modes.
        let objects = serde_json::json!([
            { "name": "a", "target": "a", "loadMode": "append", "primaryKey": ["id"] },
            { "name": "b", "target": "b", "primaryKey": ["id"] },
        ]);
        assert_eq!(validate_load_modes("sql", &objects), Ok(()));
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
    fn dial_parse_accepts_an_oracle_service_name_that_is_an_identifier() {
        // SRC-4 slice 1 review SHOULD-FIX 3.
        for name in ["FREEPDB1", "ORCLPDB1", "sales.example.com", "g6_ingest-2"] {
            let json = serde_json::json!({
                "driver": "oracle", "host": "oracle.internal", "port": 1521,
                "database": name, "user": "reader"
            });
            assert!(Dial::parse("sql", &json).is_ok(), "{name} must be accepted");
        }
    }

    #[test]
    fn dial_parse_refuses_an_oracle_service_name_that_is_not_an_identifier() {
        // SRC-4 slice 1 review SHOULD-FIX 3: a fixed sentence, never the value.
        let too_long = "A".repeat(129);
        for name in ["a)(b", "a=b", "a b", "(a", "", too_long.as_str()] {
            let json = serde_json::json!({
                "driver": "oracle", "host": "oracle.internal", "port": 1521,
                "database": name, "user": "reader"
            });
            let msg = Dial::parse("sql", &json).unwrap_err().to_string();
            assert!(msg.contains("Oracle service name"), "{name:?}: {msg}");
            assert!(
                name.is_empty() || !msg.contains(name),
                "the message must not repeat the value: {msg}"
            );
        }
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

    use super::{
        Dial, InvalidExcludedColumns, InvalidLoadMode, SourceObject, inactive_exclusions,
        is_repoint, supports_column_choice, validate_excluded_columns, validate_load_modes,
    };

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

    /// The target identity of a `cdc` dial is its driver, host, port and database, and nothing
    /// else. `SRC-4` slice 3 (decision S3-a) makes `slotName`/`publicationName` PostgreSQL-only and
    /// `serverId` MySQL-only, so this test's old fixture (a PostgreSQL dial given a `serverId` and
    /// switched to `mysql` with slot names) is no longer a dial `Dial::parse` accepts. The intent is
    /// unchanged: each field below is varied on a dial that is valid for its driver.
    #[test]
    fn a_cdc_dial_is_re_pointed_by_driver_host_port_or_database_only() {
        let base = json!({
            "driver": "postgres", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "reader",
            "slotName": "slot_a", "publicationName": "pub_a"
        });
        assert!(!same("cdc", &base, "host", json!("other.internal")));
        assert!(!same("cdc", &base, "port", json!(5433)));
        assert!(!same("cdc", &base, "database", json!("billing")));
        assert!(same("cdc", &base, "user", json!("someone_else")));
        assert!(same("cdc", &base, "slotName", json!("slot_b")));
        assert!(same("cdc", &base, "publicationName", json!("pub_b")));
        // The driver: the same host, port and database under another driver is another target.
        let mysql = json!({
            "driver": "mysql", "host": "db.internal", "port": 5432,
            "database": "orders", "user": "reader", "serverId": 7
        });
        assert_ne!(
            parse("cdc", &base).target_identity(),
            parse("cdc", &mysql).target_identity()
        );
        // MySQL's server id is not part of the target.
        assert!(same("cdc", &mysql, "serverId", json!(8)));
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

    fn sql_dial_of(driver: &str) -> Dial {
        Dial::parse(
            "sql",
            &serde_json::json!({
                "driver": driver, "host": "db.internal", "port": 5432,
                "database": "orders", "user": "app_reader"
            }),
        )
        .expect("valid sql dial")
    }

    /// `SRC-11` D9 and `SRC-4` T1.2: column choice for the four SQL drivers
    /// that list columns before loading (Oracle since `SRC-4`); a refusal for
    /// change capture and the rest.
    #[test]
    fn excluded_columns_are_accepted_only_for_sources_that_list_their_columns() {
        let objects = serde_json::json!([
            { "name": "public.orders", "target": "orders", "excludedColumns": ["note"] }
        ]);
        for driver in ["postgres", "mysql", "mssql", "oracle"] {
            assert_eq!(
                validate_excluded_columns(&sql_dial_of(driver), &objects),
                Ok(()),
                "{driver}"
            );
        }
        let refused = Err(InvalidExcludedColumns::NotForSource {
            name: "public.orders".to_owned(),
        });
        let cdc = Dial::parse(
            "cdc",
            &serde_json::json!({
                "driver": "postgres", "host": "db.internal", "port": 5432,
                "database": "orders", "user": "app_reader",
                "slotName": "orders_slot", "publicationName": "orders_pub"
            }),
        )
        .expect("valid cdc dial");
        assert_eq!(validate_excluded_columns(&cdc, &objects), refused);
        assert!(supports_column_choice(&sql_dial_of("postgres")));
        assert!(supports_column_choice(&sql_dial_of("oracle")));
        assert!(!supports_column_choice(&cdc));
    }

    /// `SRC-3` T1.3, AD6: a `saas` object takes the deduplicating incremental
    /// mode and column choice when it has a cursor; without one it is still
    /// refused, with the same message as a SQL object. `rest` is still refused.
    /// Final review BLOCKER 18: plain `incremental` is refused for `saas`
    /// because it appends a changed record as a second row.
    #[test]
    fn saas_objects_take_incremental_loads_and_column_choice_only_with_a_cursor() {
        let incremental = serde_json::json!([{
            "name": "tickets", "target": "t", "loadMode": "incremental_dedup",
            "incrementalKey": "updated_at", "primaryKey": ["id"]
        }]);
        assert_eq!(validate_load_modes("saas", &incremental), Ok(()));
        let append_incremental = serde_json::json!([{
            "name": "tickets", "target": "t", "loadMode": "incremental",
            "incrementalKey": "updated_at", "primaryKey": ["id"]
        }]);
        assert_eq!(
            validate_load_modes("saas", &append_incremental),
            Err(InvalidLoadMode::SaasNeedsMerge {
                name: "tickets".to_owned()
            })
        );
        assert_eq!(
            validate_load_modes("sql", &append_incremental),
            Ok(()),
            "a SQL source keeps plain incremental: the pushed-down query reads one row per row"
        );
        let no_cursor = serde_json::json!([{
            "name": "tickets", "target": "t", "loadMode": "incremental_dedup", "primaryKey": ["id"]
        }]);
        assert_eq!(
            validate_load_modes("saas", &no_cursor),
            Err(InvalidLoadMode::NoCursor {
                name: "tickets".to_owned()
            })
        );
        assert!(matches!(
            validate_load_modes("rest", &incremental),
            Err(InvalidLoadMode::NotForAdapter { .. })
        ));
        let saas = Dial::parse(
            "saas",
            &serde_json::json!({
                "vendor": "zendesk", "account": "acme",
                "auth": {"type": "integration"}
            }),
        )
        .expect("valid saas dial");
        assert!(supports_column_choice(&saas));
        let left_out = serde_json::json!([{
            "name": "tickets", "target": "t", "excludedColumns": ["subject"]
        }]);
        assert_eq!(validate_excluded_columns(&saas, &left_out), Ok(()));
        let cursor_left_out = serde_json::json!([{
            "name": "tickets", "target": "t", "loadMode": "incremental",
            "incrementalKey": "updated_at", "excludedColumns": ["updated_at"]
        }]);
        assert!(matches!(
            validate_excluded_columns(&saas, &cursor_left_out),
            Err(InvalidExcludedColumns::NamesCursorColumn { .. })
        ));
    }

    /// An object that leaves nothing out is fine for every source: the field
    /// absent, null and an empty list all mean "nothing".
    #[test]
    fn leaving_nothing_out_is_accepted_for_every_source() {
        let objects = serde_json::json!([
            { "name": "a", "target": "a" },
            { "name": "b", "target": "b", "excludedColumns": null },
            { "name": "c", "target": "c", "excludedColumns": [] },
        ]);
        assert_eq!(
            validate_excluded_columns(&sql_dial_of("oracle"), &objects),
            Ok(())
        );
        let inactive = inactive_exclusions(&objects);
        assert!(inactive.is_empty(), "{inactive:?}");
    }

    /// `SRC-11` D11: a key column of a deduplicating or history table and the
    /// cursor column cannot be left out; other columns can.
    #[test]
    fn excluded_columns_refuse_the_key_and_the_cursor() {
        let dial = sql_dial_of("postgres");
        let refuse = |object: serde_json::Value| {
            validate_excluded_columns(&dial, &serde_json::json!([object]))
        };
        for mode in ["replace_dedup", "incremental_dedup", "history"] {
            let mut object = serde_json::json!({
                "name": "t", "target": "t", "loadMode": mode,
                "primaryKey": ["id"], "excludedColumns": ["note", "id"]
            });
            if mode == "incremental_dedup" {
                object["incrementalKey"] = serde_json::json!("updated_at");
            }
            assert_eq!(
                refuse(object),
                Err(InvalidExcludedColumns::NamesKeyColumn {
                    name: "t".to_owned(),
                    column: "id".to_owned()
                }),
                "{mode}"
            );
        }
        assert_eq!(
            refuse(serde_json::json!({
                "name": "t", "target": "t", "loadMode": "incremental",
                "incrementalKey": "updated_at", "excludedColumns": ["updated_at"]
            })),
            Err(InvalidExcludedColumns::NamesCursorColumn {
                name: "t".to_owned(),
                column: "updated_at".to_owned()
            })
        );
        // A key is only a key in a mode that reads it: a plain replace may
        // leave out the column the console remembered as a detected key.
        assert_eq!(
            refuse(serde_json::json!({
                "name": "t", "target": "t", "loadMode": "replace",
                "primaryKey": ["id"], "excludedColumns": ["id"]
            })),
            Ok(())
        );
        assert_eq!(
            refuse(serde_json::json!({
                "name": "t", "target": "t", "loadMode": "incremental",
                "incrementalKey": "updated_at", "excludedColumns": ["note"]
            })),
            Ok(())
        );
    }

    #[test]
    fn excluded_columns_must_be_a_short_list_of_plain_distinct_names() {
        let dial = sql_dial_of("mysql");
        let long = "x".repeat(129);
        let many: Vec<String> = (0..2001).map(|n| format!("c{n}")).collect();
        for bad in [
            serde_json::json!("note"),
            serde_json::json!([1]),
            serde_json::json!([""]),
            serde_json::json!(["  "]),
            serde_json::json!(["a", "a"]),
            serde_json::json!(["a\nb"]),
            serde_json::json!([long]),
            serde_json::json!(many),
        ] {
            assert!(
                matches!(
                    validate_excluded_columns(
                        &dial,
                        &serde_json::json!([{ "name": "t", "target": "t", "excludedColumns": bad }])
                    ),
                    Err(InvalidExcludedColumns::BadList { .. })
                ),
                "{bad}"
            );
        }
        // Names are compared as written: a case difference is two columns.
        assert_eq!(
            validate_excluded_columns(
                &dial,
                &serde_json::json!([{ "name": "t", "target": "t", "excludedColumns": ["a", "A"] }])
            ),
            Ok(())
        );
    }

    /// `SRC-11` D10: only a table that keeps its rows gets the inactive mark.
    #[test]
    fn only_a_table_that_keeps_its_rows_marks_a_left_out_column_inactive() {
        let objects = serde_json::json!([
            { "name": "a", "target": "a", "loadMode": "append", "excludedColumns": ["x"] },
            { "name": "b", "target": "b", "loadMode": "replace", "excludedColumns": ["x"] },
            { "name": "c", "target": "c", "excludedColumns": ["x"] },
            { "name": "d", "target": "d", "loadMode": "replace_dedup",
              "primaryKey": ["id"], "excludedColumns": ["x"] },
            { "name": "e", "target": "e", "loadMode": "history",
              "primaryKey": ["id"], "excludedColumns": ["x", "y"] },
            { "name": "f", "target": "f", "loadMode": "incremental_dedup",
              "primaryKey": ["id"], "incrementalKey": "u", "excludedColumns": ["z"] },
        ]);
        assert_eq!(
            inactive_exclusions(&objects),
            vec![
                ("a".to_owned(), "x".to_owned()),
                ("e".to_owned(), "x".to_owned()),
                ("e".to_owned(), "y".to_owned()),
                ("f".to_owned(), "z".to_owned()),
            ]
        );
    }

    #[test]
    fn source_object_reads_excluded_columns_and_defaults_to_none() {
        let with: SourceObject = serde_json::from_value(serde_json::json!({
            "name": "public.orders", "target": "orders", "excludedColumns": ["note"]
        }))
        .expect("valid");
        assert_eq!(with.excluded_columns, Some(vec!["note".to_owned()]));
        let without: SourceObject = serde_json::from_value(
            serde_json::json!({ "name": "public.orders", "target": "orders" }),
        )
        .expect("valid");
        assert_eq!(without.excluded_columns, None);
    }
}

/// `SRC-12` part C: the `OAuth2` dial shapes, their addresses, and what moves
/// the target identity.
#[cfg(test)]
mod oauth_dial_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::{Dial, RestAuth, secret_field_names};
    use serde_json::{Value, json};

    fn rest(auth: &Value) -> Value {
        json!({
            "baseUrl": "https://api.example.com",
            "auth": auth,
            "pagination": { "type": "none" },
            "endpoints": [{ "path": "/items" }],
        })
    }

    fn code_auth() -> Value {
        json!({
            "type": "oauth2_authorization_code",
            "clientId": "app-1",
            "authorizeUrl": "https://login.example.com/authorize",
            "tokenUrl": "https://login.example.com/token",
            "revocationUrl": "https://login.example.com/revoke",
            "scopes": ["read", "offline_access"],
        })
    }

    #[test]
    fn an_authorization_code_dial_parses_with_its_camel_case_names() {
        let Dial::Rest(dial) = Dial::parse("rest", &rest(&code_auth())).expect("valid") else {
            panic!("a rest dial");
        };
        let RestAuth::Oauth2AuthorizationCode {
            client_id,
            revocation_url,
            scopes,
            ..
        } = dial.auth
        else {
            panic!("an authorization-code auth");
        };
        assert_eq!(client_id, "app-1");
        assert_eq!(
            revocation_url.as_deref(),
            Some("https://login.example.com/revoke")
        );
        assert_eq!(scopes, vec!["read", "offline_access"]);
    }

    #[test]
    fn the_revocation_address_and_scopes_are_optional() {
        let mut auth = code_auth();
        auth.as_object_mut()
            .expect("object")
            .remove("revocationUrl");
        auth.as_object_mut().expect("object").remove("scopes");
        assert!(Dial::parse("rest", &rest(&auth)).is_ok());
    }

    #[test]
    fn an_authorization_code_dial_needs_the_client_secret_and_the_refresh_token() {
        assert_eq!(
            secret_field_names("rest", Some("oauth2_authorization_code")),
            Some(["clientSecret", "refreshToken"].as_slice())
        );
    }

    #[test]
    fn the_client_credentials_token_address_parses_under_the_name_the_console_sends() {
        let auth = json!({ "type": "oauth2_client_credentials", "tokenUrl": "https://login.example.com/token" });
        assert!(Dial::parse("rest", &rest(&auth)).is_ok());
        // The name an earlier build accepted still parses.
        let old = json!({ "type": "oauth2_client_credentials", "token_url": "https://login.example.com/token" });
        assert!(Dial::parse("rest", &rest(&old)).is_ok());
    }

    #[test]
    fn an_address_that_is_not_a_plain_http_address_is_refused() {
        for bad in [
            "javascript:alert(1)",
            "ftp://login.example.com/token",
            "https://user:pw@login.example.com/token",
            "https://",
            "https://login.example.com/to ken",
            "login.example.com/token",
        ] {
            for field in ["authorizeUrl", "tokenUrl", "revocationUrl"] {
                let mut auth = code_auth();
                auth[field] = json!(bad);
                assert!(
                    Dial::parse("rest", &rest(&auth)).is_err(),
                    "{field} = {bad:?} must be refused"
                );
            }
        }
        let cc = json!({ "type": "oauth2_client_credentials", "tokenUrl": "file:///etc/passwd" });
        assert!(Dial::parse("rest", &rest(&cc)).is_err());
    }

    #[test]
    fn an_empty_client_id_and_a_bad_scope_are_refused() {
        let mut auth = code_auth();
        auth["clientId"] = json!("  ");
        assert!(Dial::parse("rest", &rest(&auth)).is_err());
        for scope in ["", "two words", "quo\"te", "new\nline"] {
            let mut auth = code_auth();
            auth["scopes"] = json!([scope]);
            assert!(
                Dial::parse("rest", &rest(&auth)).is_err(),
                "scope {scope:?} must be refused"
            );
        }
    }

    #[test]
    fn an_unknown_field_of_the_authorization_code_auth_is_refused() {
        let mut auth = code_auth();
        auth["clientSecret"] = json!("must-never-be-in-a-dial");
        assert!(Dial::parse("rest", &rest(&auth)).is_err());
    }

    #[test]
    fn moving_the_token_or_revocation_host_is_a_re_point_but_a_new_path_is_not() {
        let identity = |auth: &Value| {
            Dial::parse("rest", &rest(auth))
                .expect("valid")
                .target_identity()
        };
        let base = identity(&code_auth());

        let mut other_token_host = code_auth();
        other_token_host["tokenUrl"] = json!("https://evil.example.net/token");
        assert_ne!(base, identity(&other_token_host));

        let mut other_revocation_host = code_auth();
        other_revocation_host["revocationUrl"] = json!("https://evil.example.net/revoke");
        assert_ne!(base, identity(&other_revocation_host));

        let mut no_revocation = code_auth();
        no_revocation
            .as_object_mut()
            .expect("object")
            .remove("revocationUrl");
        assert_ne!(base, identity(&no_revocation));

        let mut new_path = code_auth();
        new_path["tokenUrl"] = json!("https://login.example.com/v2/token");
        new_path["scopes"] = json!(["read"]);
        new_path["authorizeUrl"] = json!("https://other.example.org/authorize");
        assert_eq!(base, identity(&new_path));

        let cc =
            |url: &str| identity(&json!({ "type": "oauth2_client_credentials", "tokenUrl": url }));
        assert_ne!(
            cc("https://login.example.com/token"),
            cc("https://evil.example.net/token")
        );
    }
}

#[cfg(test)]
mod saas_dial_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn dial(vendor: &str, account: &str, auth: &serde_json::Value) -> serde_json::Value {
        serde_json::json!({"vendor": vendor, "account": account, "auth": auth})
    }

    fn oauth() -> serde_json::Value {
        serde_json::json!({"type": "oauth2_authorization_code", "clientId": "app-id"})
    }

    fn integration() -> serde_json::Value {
        serde_json::json!({"type": "integration"})
    }

    #[test]
    fn a_saas_dial_parses_and_names_its_secret_slots_through_the_vendor_table() {
        let parsed = Dial::parse("saas", &dial("zendesk", "acme", &oauth())).unwrap();
        assert!(matches!(parsed, Dial::Saas(_)));
        assert_eq!(
            parsed.secret_map_auth_type(),
            Some("oauth2_authorization_code")
        );
        assert_eq!(
            secret_field_names("saas", parsed.secret_map_auth_type()),
            Some(["clientSecret", "refreshToken"].as_slice())
        );
        let token = Dial::parse("saas", &dial("zendesk", "acme", &integration())).unwrap();
        assert_eq!(token.secret_map_auth_type(), Some("integration:zendesk"));
        assert_eq!(
            secret_field_names("saas", token.secret_map_auth_type()),
            Some(["email", "apiToken"].as_slice())
        );
        assert_eq!(secret_field_names("saas", Some("bearer")), None);
        assert_eq!(secret_field_names("saas", None), None);
    }

    #[test]
    fn a_saas_dial_refuses_an_unknown_vendor_and_a_credential_smuggled_into_it() {
        assert!(Dial::parse("saas", &dial("nobody", "acme", &oauth())).is_err());
        let mut with_secret = dial("zendesk", "acme", &oauth());
        with_secret["password"] = serde_json::json!("hunter2");
        assert!(Dial::parse("saas", &with_secret).is_err());
        let mut with_url = dial("zendesk", "acme", &oauth());
        with_url["auth"]["tokenUrl"] = serde_json::json!("https://evil.example.invalid/token");
        assert!(Dial::parse("saas", &with_url).is_err());
    }

    #[test]
    fn a_saas_dial_refuses_an_account_outside_the_vendors_host_pattern() {
        for account in [
            "evil.example.invalid",
            "acme@evil",
            "acme:8443",
            "https://acme",
            "http://acme",
            "acme/path",
        ] {
            assert!(
                Dial::parse("saas", &dial("zendesk", account, &oauth())).is_err(),
                "{account}"
            );
        }
    }

    #[test]
    fn salesforce_offers_no_integration_credential_and_a_blank_client_id_is_refused() {
        // `SRC-3` decision 4: the password sign-in is not offered.
        assert!(Dial::parse("saas", &dial("salesforce", "acme", &integration())).is_err());
        let blank = serde_json::json!({"type": "oauth2_authorization_code", "clientId": " "});
        assert!(Dial::parse("saas", &dial("salesforce", "acme", &blank)).is_err());
    }

    #[test]
    fn saas_object_names_must_be_distinct_plain_text() {
        let mut value = dial("ga4", "123456", &oauth());
        value["objects"] = serde_json::json!(["report one", "report two"]);
        assert!(Dial::parse("saas", &value).is_ok());
        value["objects"] = serde_json::json!(["same", "same"]);
        assert!(Dial::parse("saas", &value).is_err());
        value["objects"] = serde_json::json!([" "]);
        assert!(Dial::parse("saas", &value).is_err());
        value["objects"] = serde_json::json!(["bad\nname"]);
        assert!(Dial::parse("saas", &value).is_err());
    }

    /// `SRC-3` slice 6: `NetSuite`'s client-credentials way names the certificate uploaded in `NetSuite`
    /// (the assertion's `kid`); no other vendor or sign-in has one, and it must be a plain name.
    #[test]
    fn a_certificate_id_is_for_netsuites_integration_credential_only_and_is_a_plain_name() {
        let with = |vendor: &str, account: &str, auth: &serde_json::Value, id: &str| {
            let mut value = dial(vendor, account, auth);
            value["certificateId"] = serde_json::json!(id);
            Dial::parse("saas", &value)
        };
        assert!(with("netsuite", "1234567_SB1", &integration(), "cert-1.A_b").is_ok());
        for bad in ["", "a b", "a/b", "a\nb", &"x".repeat(129)] {
            assert!(
                with("netsuite", "1234567", &integration(), bad).is_err(),
                "{bad:?}"
            );
        }
        assert!(with("netsuite", "1234567", &oauth(), "cert-1").is_err());
        assert!(with("zendesk", "acme", &integration(), "cert-1").is_err());
        // Final review SHOULD-FIX 25: the integration credential cannot be signed without the
        // certificate id, so the save refuses it now rather than the connection test later.
        assert!(Dial::parse("saas", &dial("netsuite", "1234567", &integration())).is_err());
        assert!(Dial::parse("saas", &dial("netsuite", "1234567", &oauth())).is_ok());
        // Not part of the target identity: it is a name, not a destination.
        let a = with("netsuite", "1234567", &integration(), "cert-1").unwrap();
        let b = with("netsuite", "1234567", &integration(), "cert-2").unwrap();
        assert_eq!(a.target_identity(), b.target_identity());
    }

    #[test]
    fn a_workday_dial_names_host_and_tenant_and_takes_the_integration_users_two_secrets() {
        let parsed = Dial::parse(
            "saas",
            &dial(
                "workday",
                "wd2-impl-services1.workday.com/acme_tenant",
                &integration(),
            ),
        )
        .unwrap();
        assert_eq!(
            secret_field_names("saas", parsed.secret_map_auth_type()),
            Some(["username", "password"].as_slice())
        );
        assert!(Dial::parse("saas", &dial("workday", "wd2.workday.com", &integration())).is_err());
    }

    #[test]
    fn the_saas_target_identity_changes_with_the_vendor_the_account_or_the_sign_in_kind() {
        let base = Dial::parse("saas", &dial("zendesk", "acme", &oauth())).unwrap();
        let same_account_other_case =
            Dial::parse("saas", &dial("zendesk", "ACME", &oauth())).unwrap();
        assert_eq!(
            base.target_identity(),
            same_account_other_case.target_identity()
        );
        // Final review SHOULD-FIX 25: a different sign-in kind is a different role for the same
        // stored slots, so the credentials must be asked again (this used to assert equality).
        let other_auth = Dial::parse("saas", &dial("zendesk", "acme", &integration())).unwrap();
        assert_ne!(base.target_identity(), other_auth.target_identity());
        let mut renamed = dial("zendesk", "acme", &oauth());
        renamed["objects"] = serde_json::json!(["tickets"]);
        renamed["auth"]["clientId"] = serde_json::json!("another-app");
        let renamed = Dial::parse("saas", &renamed).unwrap();
        assert_eq!(base.target_identity(), renamed.target_identity());
        let other_account = Dial::parse("saas", &dial("zendesk", "other", &oauth())).unwrap();
        assert_ne!(base.target_identity(), other_account.target_identity());
        let other_vendor = Dial::parse("saas", &dial("confluence", "acme", &oauth())).unwrap();
        assert_ne!(base.target_identity(), other_vendor.target_identity());
        assert!(is_repoint(
            Some("saas"),
            &dial("zendesk", "acme", &oauth()),
            &other_account
        ));
        assert!(!is_repoint(
            Some("saas"),
            &dial("zendesk", "acme", &oauth()),
            &renamed
        ));
        assert!(is_repoint(
            Some("saas"),
            &dial("zendesk", "acme", &oauth()),
            &other_auth
        ));
    }
}
