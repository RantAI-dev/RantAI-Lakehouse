//! The fixed table of business-app (`SaaS`) vendors (`SRC-3`, plan AD1 and AD2,
//! `docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md`).
//!
//! # Why the hosts are built, never typed (AD2)
//!
//! A `saas` connector's API, sign-in and token hosts come from this table.
//! The person supplies only the vendor's per-customer part (`account`: a
//! Zendesk subdomain, a `ServiceNow` instance, a Dataverse environment...),
//! and [`SaasVendor::validate_account`] accepts it only in the shape the
//! vendor's host pattern allows. A value with a scheme, a port, a path
//! (other than Workday's `host/tenant`), userinfo or a dot the pattern does
//! not expect is refused, so the typed text cannot move a request to
//! another host (`SEC-14`, `SEC-15`). The sign-in and token addresses are
//! part of the table too, so a dial cannot override them (AD8).
//!
//! # One table in three places
//!
//! `ops/fixtures/saas_vendors.json` is the source of truth. This table, the
//! orchestrator's `saas/vendors` registry and the console read the same
//! file's content; each has a test that compares its own copy with the file,
//! so a vendor changed in one place and not the others fails a test.
//!
//! This module sits in `lakehouse-store`, not in the API crate, because
//! [`crate::ingest_spec::Dial::parse`] validates the account against it and
//! the API crate depends on this one.

use serde::Serialize;

/// How a vendor's `account` text is validated and turned into host parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AccountRule {
    /// One DNS label: letters, digits and hyphens, not starting or ending
    /// with a hyphen, 63 characters at most.
    Label,
    /// Digits only (a property or account number), 20 at most.
    Digits,
    /// A vendor account id: letters, digits, `-` and `_`, starting and ending
    /// with a letter or digit, 40 at most. The host form lower-cases it and
    /// writes `_` as `-` (`NetSuite`: `1234567_SB1` is host `1234567-sb1`).
    AccountId,
    /// A Dataverse environment address `<org>.crm<N>.dynamics.com`, where
    /// `<N>` is zero to two digits (the vendor's region number).
    DataverseHost,
    /// SharePoint (slice 5): `<label>/<tenant>`, a [`Self::Label`] and the customer's Microsoft
    /// Entra tenant (see [`is_entra_tenant`]). The tenant forms the token address.
    LabelTenant,
    /// Dynamics 365 (slice 5): `<org>.crm<N>.dynamics.com/<tenant>`, a [`Self::DataverseHost`]
    /// and the customer's Microsoft Entra tenant.
    DataverseHostTenant,
    /// `<host>/<tenant>`: `host` is one label under one of `suffixes`, and
    /// `tenant` a name of letters, digits, `-` and `_`.
    HostTenant {
        /// The domains the host must end with, each with its leading dot.
        suffixes: &'static [&'static str],
    },
}

/// How the token request carries the client's credentials (RFC 6749 2.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientAuth {
    /// `client_id` and `client_secret` in the form body (`SRC-12`'s only
    /// method until `SRC-3`).
    Post,
    /// HTTP Basic `Authorization` header (`SRC-3` AD8: `NetSuite` needs it).
    Basic,
    /// `client_secret_post_json`: the same fields as `Post`, as a JSON object body
    /// (slice 2 review BLOCKER 3: Zendesk's `POST /oauth/tokens` "must be formatted
    /// as JSON", <https://developer.zendesk.com/api-reference/ticketing/oauth/grant_type_tokens/>).
    PostJson,
}

/// A vendor's sign-in in the browser (OAuth authorisation code with PKCE,
/// `SRC-12`), as fixed addresses with placeholders. Decision 3.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthPreset {
    /// Opened by the person's browser, never dialled by the product.
    pub authorize_url: &'static str,
    /// Dialled by the product; receives the client secret and refresh token.
    pub token_url: &'static str,
    /// The revocation endpoint (RFC 7009), when the vendor has one.
    pub revocation_url: Option<&'static str>,
    /// The scopes asked for; `{account}` is replaced like in the addresses.
    pub scopes: &'static [&'static str],
    /// How the token request carries the client's credentials.
    pub client_auth: ClientAuth,
    /// The API's base address for a browser sign-in, when it is not [`SaasVendor::api_base`]:
    /// Atlassian's OAuth tokens are accepted only at `api.atlassian.com/ex/confluence/{cloudId}`,
    /// not at the customer's `<site>.atlassian.net`. A fixed vendor host from this table (AD2),
    /// never one taken from a response; `{cloudId}` is the one placeholder that only the
    /// orchestrator fills, after the sign-in, from the vendor's list of accessible sites.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_base: Option<&'static str>,
}

/// The second way to connect: the vendor's integration credential
/// (decision 3), as the secret slots it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Integration {
    /// The `auth_type` key [`crate::ingest_spec::secret_field_names`] looks
    /// the slots up by: `integration:<vendor id>`.
    pub secret_key: &'static str,
    /// The named secret fields, in slot order (primary first).
    pub secret_fields: &'static [&'static str],
    /// The `scope` of the client-credentials request, with the same placeholders as the
    /// addresses, when the vendor needs one: Microsoft Entra wants the resource's `/.default`
    /// (<https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow>).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<&'static str>,
}

/// The vendor's "who am I" request used by the connection test.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Whoami {
    /// The HTTP method; `GET` for every vendor so far.
    pub method: &'static str,
    /// The path (and query) under [`SaasVendor::api_base`].
    pub path: &'static str,
}

/// One business-app vendor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaasVendor {
    /// The `vendor` value of a `saas` dial.
    pub id: &'static str,
    /// The name shown to a person.
    pub label: &'static str,
    /// The `connector_type.name` this vendor is listed under
    /// (`0095_saas_connector_types.sql`).
    pub connector_type: &'static str,
    /// How `account` is validated.
    pub account: AccountRule,
    /// What the console asks for in the account field.
    pub account_label: &'static str,
    /// The API's base address, with placeholders.
    pub api_base: &'static str,
    /// Sign-in in the browser.
    pub oauth: OauthPreset,
    /// The integration credential, when the vendor offers one.
    pub integration: Option<Integration>,
    /// The connection test's request, when the vendor has a generic one.
    pub whoami: Option<Whoami>,
}

/// A validated account split into the parts the address placeholders use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountParts {
    /// Replaces `{account}`: the account in host form (lower case).
    pub account: String,
    /// Replaces `{host}` (empty unless the vendor's rule is `HostTenant`).
    pub host: String,
    /// Replaces `{tenant}` (empty unless the vendor's rule is `HostTenant`).
    pub tenant: String,
}

/// Why an `account` was refused. The texts are fixed and never echo the
/// input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidAccount(&'static str);

/// The `auth_type` key of a `saas` dial that signs in in the browser.
pub const OAUTH_AUTH_TYPE: &str = "oauth2_authorization_code";

/// The secret fields of a browser sign-in: the client secret (primary) and
/// the refresh token the callback writes (secondary), as for a `rest`
/// connector (`SRC-12` part C).
pub const OAUTH_SECRET_FIELDS: &[&str] = &["clientSecret", "refreshToken"];

fn is_label(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 63
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

fn is_tenant(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Whether `text` is a Microsoft Entra tenant as `login.microsoftonline.com/{tenant}` takes it, in
/// "GUID or domain-name format"
/// (<https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-client-creds-grant-flow>).
/// `common`, `organizations` and `consumers` are not one customer's tenant and have no dot, so a
/// domain needs two labels or more; its last label must hold a letter, so a dotted number is not
/// taken for a domain. Only these characters can reach the token address' path.
fn is_entra_tenant(text: &str) -> bool {
    let is_guid = text.len() == 36
        && text.char_indices().all(|(index, c)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
    if is_guid {
        return true;
    }
    let labels: Vec<&str> = text.split('.').collect();
    text.len() <= 253
        && labels.len() >= 2
        && labels.iter().all(|label| is_label(label))
        && labels
            .last()
            .is_some_and(|last| last.chars().any(|c| c.is_ascii_alphabetic()))
}

impl SaasVendor {
    /// Validate `account` against this vendor's rule and split it into the
    /// parts the placeholders use.
    ///
    /// The text is taken exactly as given: whitespace, a scheme, userinfo, a
    /// port or a path all fail the character checks, so none can reach a
    /// built address.
    ///
    /// # Errors
    ///
    /// [`InvalidAccount`] with a fixed text when `account` does not have the
    /// shape the vendor's host pattern allows.
    pub fn validate_account(&self, account: &str) -> Result<AccountParts, InvalidAccount> {
        let bad = InvalidAccount("account is not in the form this vendor uses");
        let plain = |account: String| AccountParts {
            account,
            host: String::new(),
            tenant: String::new(),
        };
        match self.account {
            AccountRule::Label => is_label(account)
                .then(|| plain(account.to_ascii_lowercase()))
                .ok_or(bad),
            AccountRule::Digits => (!account.is_empty()
                && account.len() <= 20
                && account.chars().all(|c| c.is_ascii_digit()))
            .then(|| plain(account.to_owned()))
            .ok_or(bad),
            AccountRule::AccountId => {
                let ok = !account.is_empty()
                    && account.len() <= 40
                    && account.starts_with(|c: char| c.is_ascii_alphanumeric())
                    && account.ends_with(|c: char| c.is_ascii_alphanumeric())
                    && account
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                ok.then(|| plain(account.to_ascii_lowercase().replace('_', "-")))
                    .ok_or(bad)
            }
            AccountRule::DataverseHost => {
                let lowered = account.to_ascii_lowercase();
                let region_org = lowered.strip_suffix(".dynamics.com").ok_or(bad)?;
                let (org, region) = region_org.rsplit_once('.').ok_or(bad)?;
                let digits = region.strip_prefix("crm").ok_or(bad)?;
                let ok = digits.len() <= 2
                    && digits.chars().all(|c| c.is_ascii_digit())
                    && is_label(org);
                ok.then(|| plain(lowered)).ok_or(bad)
            }
            AccountRule::LabelTenant | AccountRule::DataverseHostTenant => {
                // Slice 5: `<account>/<tenant>`; the account is checked by the plain rule.
                let (inner, tenant) = account.split_once('/').ok_or(bad)?;
                if !is_entra_tenant(tenant) {
                    return Err(bad);
                }
                let base = if self.account == AccountRule::LabelTenant {
                    AccountRule::Label
                } else {
                    AccountRule::DataverseHost
                };
                let mut parts = SaasVendor {
                    account: base,
                    ..*self
                }
                .validate_account(inner)?;
                parts.tenant = tenant.to_ascii_lowercase();
                Ok(parts)
            }
            AccountRule::HostTenant { suffixes } => {
                let (host, tenant) = account.split_once('/').ok_or(bad)?;
                let lowered = host.to_ascii_lowercase();
                let label_ok = suffixes
                    .iter()
                    .any(|suffix| lowered.strip_suffix(suffix).is_some_and(is_label));
                (label_ok && is_tenant(tenant))
                    .then(|| AccountParts {
                        account: format!("{lowered}/{tenant}"),
                        host: lowered,
                        tenant: tenant.to_owned(),
                    })
                    .ok_or(bad)
            }
        }
    }

    /// `template` with `{account}`, `{host}` and `{tenant}` replaced.
    #[must_use]
    pub fn render(template: &str, parts: &AccountParts) -> String {
        template
            .replace("{account}", &parts.account)
            .replace("{host}", &parts.host)
            .replace("{tenant}", &parts.tenant)
    }

    /// The `auth_type` key the secret mapping uses for a connector of this
    /// vendor that connects with `oauth` (browser sign-in) or the
    /// integration credential. `None` when the vendor has no integration
    /// credential and `oauth` is false.
    #[must_use]
    pub fn auth_type_key(&self, oauth: bool) -> Option<&'static str> {
        if oauth {
            Some(OAUTH_AUTH_TYPE)
        } else {
            self.integration.map(|integration| integration.secret_key)
        }
    }
}

/// The vendor whose `id` is `id`.
#[must_use]
pub fn vendor(id: &str) -> Option<&'static SaasVendor> {
    SAAS_VENDORS.iter().find(|vendor| vendor.id == id)
}

/// The secret fields behind a `saas` `auth_type` key: the browser sign-in's,
/// or a vendor's integration credential's. `None` for a key no vendor has.
#[must_use]
pub fn secret_fields_for(auth_type: &str) -> Option<&'static [&'static str]> {
    if auth_type == OAUTH_AUTH_TYPE {
        return Some(OAUTH_SECRET_FIELDS);
    }
    SAAS_VENDORS
        .iter()
        .filter_map(|vendor| vendor.integration)
        .find(|integration| integration.secret_key == auth_type)
        .map(|integration| integration.secret_fields)
}

/// The ten vendors of `SRC-3`'s first wave, in the build order of decision
/// 12 after the shared parts. Compared with
/// `ops/fixtures/saas_vendors.json` by a test.
pub const SAAS_VENDORS: &[SaasVendor] = &[
    SaasVendor {
        id: "salesforce",
        label: "Salesforce",
        connector_type: "Salesforce",
        account: AccountRule::Label,
        account_label: "My Domain name (the part before .my.salesforce.com)",
        api_base: "https://{account}.my.salesforce.com",
        oauth: OauthPreset {
            authorize_url: "https://{account}.my.salesforce.com/services/oauth2/authorize",
            token_url: "https://{account}.my.salesforce.com/services/oauth2/token",
            revocation_url: Some("https://{account}.my.salesforce.com/services/oauth2/revoke"),
            scopes: &["api", "refresh_token"],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: None,
        whoami: Some(Whoami {
            method: "GET",
            path: "/services/data/v62.0/limits",
        }),
    },
    SaasVendor {
        id: "servicenow",
        label: "ServiceNow",
        connector_type: "ServiceNow",
        account: AccountRule::Label,
        account_label: "Instance name (the part before .service-now.com)",
        api_base: "https://{account}.service-now.com",
        oauth: OauthPreset {
            authorize_url: "https://{account}.service-now.com/oauth_auth.do",
            token_url: "https://{account}.service-now.com/oauth_token.do",
            // ServiceNow revokes by GET `oauth_revoke_token.do?token=...`, not RFC 7009's POST
            // (<https://www.servicenow.com/docs/r/8yND1KuvF7tjP~F577YThQ/hMCUQrOe2CEAjaIJdG03MA>).
            revocation_url: None,
            scopes: &[],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:servicenow",
            secret_fields: &["clientId", "clientSecret"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/api/now/table/sys_user?sysparm_limit=1&sysparm_fields=sys_id",
        }),
    },
    SaasVendor {
        id: "workday",
        label: "Workday Reports",
        connector_type: "Workday Reports",
        account: AccountRule::HostTenant {
            suffixes: &[".workday.com", ".myworkday.com"],
        },
        account_label: "Workday host and tenant, as host/tenant",
        api_base: "https://{host}",
        oauth: OauthPreset {
            authorize_url: "https://{host}/{tenant}/authorize",
            token_url: "https://{host}/ccx/oauth2/{tenant}/token",
            revocation_url: None,
            scopes: &[],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:workday",
            secret_fields: &["username", "password"],
            scope: None,
        }),
        whoami: None,
    },
    SaasVendor {
        id: "ga4",
        label: "Google Analytics 4",
        connector_type: "Google Analytics 4",
        account: AccountRule::Digits,
        account_label: "GA4 property id (digits)",
        api_base: "https://analyticsdata.googleapis.com",
        oauth: OauthPreset {
            authorize_url: "https://accounts.google.com/o/oauth2/v2/auth?access_type=offline&prompt=consent",
            token_url: "https://oauth2.googleapis.com/token",
            revocation_url: Some("https://oauth2.googleapis.com/revoke"),
            scopes: &["https://www.googleapis.com/auth/analytics.readonly"],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:ga4",
            secret_fields: &["serviceAccountJson"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/v1beta/properties/{account}/metadata",
        }),
    },
    SaasVendor {
        id: "hubspot",
        label: "HubSpot",
        connector_type: "HubSpot",
        account: AccountRule::Digits,
        account_label: "HubSpot account id (digits)",
        api_base: "https://api.hubapi.com",
        oauth: OauthPreset {
            authorize_url: "https://app.hubspot.com/oauth/authorize",
            token_url: "https://api.hubapi.com/oauth/v1/token",
            revocation_url: None,
            scopes: &[
                "crm.objects.companies.read",
                "crm.objects.contacts.read",
                "crm.objects.deals.read",
                "crm.objects.owners.read",
                "crm.objects.quotes.read",
                "crm.schemas.companies.read",
                "crm.schemas.contacts.read",
                "crm.schemas.deals.read",
                "crm.schemas.quotes.read",
                "tickets",
                "e-commerce",
                "oauth",
            ],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:hubspot",
            secret_fields: &["token"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/account-info/v3/details",
        }),
    },
    SaasVendor {
        id: "zendesk",
        label: "Zendesk",
        connector_type: "Zendesk",
        account: AccountRule::Label,
        account_label: "Subdomain (the part before .zendesk.com)",
        api_base: "https://{account}.zendesk.com",
        oauth: OauthPreset {
            authorize_url: "https://{account}.zendesk.com/oauth/authorizations/new",
            token_url: "https://{account}.zendesk.com/oauth/tokens",
            revocation_url: None,
            scopes: &["read"],
            client_auth: ClientAuth::PostJson,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:zendesk",
            secret_fields: &["email", "apiToken"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/api/v2/users/me.json",
        }),
    },
    SaasVendor {
        id: "confluence",
        label: "Confluence",
        connector_type: "Confluence",
        account: AccountRule::Label,
        account_label: "Site name (the part before .atlassian.net)",
        api_base: "https://{account}.atlassian.net",
        oauth: OauthPreset {
            // `audience` and `prompt=consent` are required by Atlassian 3LO
            // (<https://developer.atlassian.com/cloud/confluence/oauth-2-3lo-apps/>).
            authorize_url: "https://auth.atlassian.com/authorize?audience=api.atlassian.com&prompt=consent",
            token_url: "https://auth.atlassian.com/oauth/token",
            revocation_url: None,
            scopes: &[
                "read:page:confluence",
                "read:space:confluence",
                "read:comment:confluence",
                "offline_access",
            ],
            // Atlassian's token endpoint takes a JSON body (same page).
            client_auth: ClientAuth::PostJson,
            api_base: Some("https://api.atlassian.com/ex/confluence/{cloudId}"),
        },
        integration: Some(Integration {
            secret_key: "integration:confluence",
            secret_fields: &["email", "apiToken"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/wiki/api/v2/spaces?limit=1",
        }),
    },
    SaasVendor {
        id: "dynamics365",
        label: "Dynamics 365",
        connector_type: "Dynamics 365",
        account: AccountRule::DataverseHostTenant,
        account_label: "Environment address and tenant, as org.crm4.dynamics.com/tenant-id-or-domain",
        api_base: "https://{account}",
        oauth: OauthPreset {
            // The customer's tenant, not `organizations`: a single-tenant app registration cannot sign in
            // through the common endpoints
            // (<https://learn.microsoft.com/en-us/entra/identity-platform/v2-oauth2-auth-code-flow>).
            // Microsoft has no RFC 7009 revocation endpoint in the v2 platform: none is listed.
            authorize_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
            revocation_url: None,
            scopes: &["https://{account}/user_impersonation", "offline_access"],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:dynamics365",
            secret_fields: &["clientId", "clientSecret"],
            scope: Some("https://{account}/.default"),
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/api/data/v9.2/WhoAmI",
        }),
    },
    SaasVendor {
        id: "netsuite",
        label: "NetSuite",
        connector_type: "NetSuite",
        account: AccountRule::AccountId,
        account_label: "Account id (for example 1234567 or 1234567_SB1)",
        api_base: "https://{account}.suitetalk.api.netsuite.com",
        oauth: OauthPreset {
            authorize_url: "https://{account}.app.netsuite.com/app/login/oauth2/authorize.nl",
            token_url: "https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/token",
            revocation_url: Some(
                "https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/revoke",
            ),
            scopes: &["rest_webservices"],
            client_auth: ClientAuth::Basic,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:netsuite",
            secret_fields: &["clientId", "privateKey"],
            scope: None,
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/services/rest/record/v1/employee?limit=1",
        }),
    },
    SaasVendor {
        id: "sharepoint",
        label: "SharePoint",
        connector_type: "SharePoint",
        account: AccountRule::LabelTenant,
        account_label: "Tenant name and tenant, as contoso/tenant-id-or-domain (contoso is the part before .sharepoint.com)",
        api_base: "https://graph.microsoft.com",
        oauth: OauthPreset {
            authorize_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
            token_url: "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
            revocation_url: None,
            scopes: &[
                "https://graph.microsoft.com/Sites.Read.All",
                "offline_access",
            ],
            client_auth: ClientAuth::Post,
            api_base: None,
        },
        integration: Some(Integration {
            secret_key: "integration:sharepoint",
            secret_fields: &["clientId", "clientSecret"],
            scope: Some("https://graph.microsoft.com/.default"),
        }),
        whoami: Some(Whoami {
            method: "GET",
            path: "/v1.0/sites/{account}.sharepoint.com",
        }),
    },
];

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn sample(id: &str) -> &'static str {
        match id {
            "salesforce" | "servicenow" | "zendesk" | "confluence" => "acme",
            "sharepoint" => "acme/00000000-0000-0000-0000-000000000001",
            "workday" => "wd2-impl-services1.workday.com/acme_tenant",
            "ga4" | "hubspot" => "123456789",
            "dynamics365" => "acme.crm4.dynamics.com/contoso.example.invalid",
            "netsuite" => "1234567_SB1",
            other => panic!("no sample account for {other}"),
        }
    }

    #[test]
    fn the_table_equals_the_shared_fixture() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../../../ops/fixtures/saas_vendors.json"))
                .unwrap();
        let ours = serde_json::to_value(SAAS_VENDORS).unwrap();
        assert_eq!(ours, fixture["vendors"]);
    }

    #[test]
    fn ids_and_connector_types_are_unique_and_there_are_ten() {
        let mut ids: Vec<_> = SAAS_VENDORS.iter().map(|v| v.id).collect();
        let mut types: Vec<_> = SAAS_VENDORS.iter().map(|v| v.connector_type).collect();
        ids.sort_unstable();
        ids.dedup();
        types.sort_unstable();
        types.dedup();
        assert_eq!(ids.len(), 10);
        assert_eq!(types.len(), 10);
    }

    #[test]
    fn every_vendor_accepts_a_good_account_and_builds_https_addresses_only() {
        for vendor in SAAS_VENDORS {
            let parts = vendor.validate_account(sample(vendor.id)).unwrap();
            for template in [
                vendor.api_base,
                vendor.oauth.authorize_url,
                vendor.oauth.token_url,
            ]
            .into_iter()
            .chain(vendor.oauth.revocation_url)
            {
                let url = SaasVendor::render(template, &parts);
                assert!(url.starts_with("https://"), "{}: {url}", vendor.id);
                assert!(!url.contains(['{', '}']), "{}: {url}", vendor.id);
            }
            for scope in vendor.oauth.scopes {
                let scope = SaasVendor::render(scope, &parts);
                assert!(!scope.contains(['{', '}', ' ']), "{}: {scope}", vendor.id);
            }
        }
    }

    #[test]
    fn every_vendor_refuses_a_value_outside_its_host_pattern() {
        let bad = [
            "evil.example.invalid",
            "https://acme",
            "http://acme",
            "acme@evil",
            "user:pass@acme",
            "acme:8443",
            "acme/path",
            "acme.evil",
            " acme",
            "acme ",
            "",
            "-acme",
            "acme-",
            "acme\n",
            "ac me",
        ];
        for vendor in SAAS_VENDORS {
            for value in bad {
                assert!(
                    vendor.validate_account(value).is_err(),
                    "{} accepted {value:?}",
                    vendor.id
                );
            }
        }
    }

    #[test]
    fn workday_dataverse_and_netsuite_refuse_their_own_near_misses() {
        let workday = vendor("workday").unwrap();
        for value in [
            "wd2.workday.com",
            "wd2.workday.com/",
            "wd2.workday.com/a/b",
            "a.b.workday.com/t",
            "wd2.workday.com.evil.invalid/t",
            "wd2.example.invalid/t",
            "https://wd2.workday.com/t",
            "wd2.workday.com:443/t",
        ] {
            assert!(workday.validate_account(value).is_err(), "{value:?}");
        }
        let dataverse = vendor("dynamics365").unwrap();
        for value in [
            "acme.dynamics.com",
            "acme.crm123.dynamics.com",
            "acme.crmx.dynamics.com",
            "acme.crm.dynamics.com.evil.invalid",
            "a.b.crm.dynamics.com",
            "acme.api.crm.dynamics.com",
        ] {
            let account = format!("{value}/00000000-0000-0000-0000-000000000001");
            assert!(dataverse.validate_account(&account).is_err(), "{value:?}");
        }
        assert!(
            dataverse
                .validate_account("Acme.CRM.dynamics.com/00000000-0000-0000-0000-000000000001")
                .is_ok_and(|parts| parts.account == "acme.crm.dynamics.com")
        );
        let netsuite = vendor("netsuite").unwrap();
        assert!(netsuite.validate_account("1234567_").is_err());
        assert!(netsuite.validate_account("_1234567").is_err());
        assert_eq!(
            netsuite.validate_account("1234567_SB1").unwrap().account,
            "1234567-sb1"
        );
    }

    #[test]
    fn a_microsoft_account_carries_a_tenant_that_forms_the_token_address() {
        for (id, inner) in [
            ("sharepoint", "Acme"),
            ("dynamics365", "Acme.CRM4.dynamics.com"),
        ] {
            let vendor = vendor(id).unwrap();
            for tenant in [
                "00000000-0000-0000-0000-00000000000A",
                "contoso.example.invalid",
                "a.b.example.invalid",
            ] {
                let parts = vendor
                    .validate_account(&format!("{inner}/{tenant}"))
                    .unwrap();
                assert_eq!(parts.tenant, tenant.to_ascii_lowercase());
                assert_eq!(parts.account, inner.to_ascii_lowercase());
                assert_eq!(
                    SaasVendor::render(vendor.oauth.token_url, &parts),
                    format!(
                        "https://login.microsoftonline.com/{}/oauth2/v2.0/token",
                        tenant.to_ascii_lowercase()
                    )
                );
            }
        }
    }

    #[test]
    fn a_tenant_that_is_not_a_guid_or_a_domain_name_is_refused() {
        let long_label = format!("{}.com", "x".repeat(64));
        for id in ["sharepoint", "dynamics365"] {
            let vendor = vendor(id).unwrap();
            let inner = if id == "sharepoint" {
                "acme"
            } else {
                "acme.crm4.dynamics.com"
            };
            for tenant in [
                "",
                "common",
                "organizations",
                "consumers",
                "contoso",
                "contoso.",
                ".contoso.com",
                "1.2.3.4",
                "a..b.com",
                "-a.example.invalid",
                "contoso.example.invalid/x",
                "contoso.example.invalid?x=1",
                "contoso.example.invalid#x",
                "contoso.example.invalid:443",
                "user@contoso.example.invalid",
                "../x.example.invalid",
                "00000000-0000-0000-0000-00000000000g",
                "00000000000000000000000000000001",
                "0000000-00000-0000-0000-000000000001",
                " contoso.example.invalid",
                long_label.as_str(),
            ] {
                assert!(
                    vendor
                        .validate_account(&format!("{inner}/{tenant}"))
                        .is_err(),
                    "{id}: {tenant:?}"
                );
            }
            // No tenant at all.
            for account in [
                inner,
                &format!("{inner}/"),
                "/00000000-0000-0000-0000-000000000001",
            ] {
                assert!(
                    vendor.validate_account(account).is_err(),
                    "{id}: {account:?}"
                );
            }
        }
    }

    #[test]
    fn only_the_two_microsoft_vendors_name_a_client_credentials_scope() {
        let scoped: Vec<_> = SAAS_VENDORS
            .iter()
            .filter(|v| v.integration.is_some_and(|i| i.scope.is_some()))
            .map(|v| v.id)
            .collect();
        assert_eq!(scoped, ["dynamics365", "sharepoint"]);
        for vendor in SAAS_VENDORS {
            if let Some(scope) = vendor.integration.and_then(|i| i.scope) {
                assert!(scope.ends_with("/.default"), "{}: {scope}", vendor.id);
            }
        }
    }

    #[test]
    fn the_account_is_rendered_into_the_address_in_lower_case() {
        let zendesk = vendor("zendesk").unwrap();
        let parts = zendesk.validate_account("AcMe").unwrap();
        assert_eq!(
            SaasVendor::render(zendesk.api_base, &parts),
            "https://acme.zendesk.com"
        );
    }

    #[test]
    fn integration_keys_resolve_to_their_vendors_slots_and_oauth_to_two() {
        for vendor in SAAS_VENDORS {
            if let Some(integration) = vendor.integration {
                assert_eq!(
                    secret_fields_for(integration.secret_key),
                    Some(integration.secret_fields),
                    "{}",
                    vendor.id
                );
                assert!(integration.secret_key.ends_with(vendor.id));
            }
        }
        assert_eq!(
            secret_fields_for(OAUTH_AUTH_TYPE),
            Some(["clientSecret", "refreshToken"].as_slice())
        );
        assert_eq!(secret_fields_for("integration:nobody"), None);
        // Decision 4: Salesforce's password sign-in is not offered.
        assert!(vendor("salesforce").unwrap().integration.is_none());
    }

    #[test]
    fn only_netsuite_signs_in_with_basic_client_authentication() {
        let basic: Vec<_> = SAAS_VENDORS
            .iter()
            .filter(|v| v.oauth.client_auth == ClientAuth::Basic)
            .map(|v| v.id)
            .collect();
        assert_eq!(basic, ["netsuite"]);
    }

    #[test]
    fn only_confluence_has_its_own_browser_api_address_and_it_is_a_fixed_https_host() {
        let own: Vec<_> = SAAS_VENDORS
            .iter()
            .filter(|v| v.oauth.api_base.is_some())
            .map(|v| v.id)
            .collect();
        assert_eq!(own, ["confluence"]);
        let base = vendor("confluence").unwrap().oauth.api_base.unwrap();
        assert!(base.starts_with("https://api.atlassian.com/ex/confluence/"));
        // `{cloudId}` is the only placeholder, and the host part has none.
        assert_eq!(base.matches('{').count(), 1);
        assert!(base.ends_with("/{cloudId}"));
    }

    #[test]
    fn only_zendesk_and_confluence_sign_in_with_a_json_token_request() {
        let json: Vec<_> = SAAS_VENDORS
            .iter()
            .filter(|v| v.oauth.client_auth == ClientAuth::PostJson)
            .map(|v| v.id)
            .collect();
        assert_eq!(json, ["zendesk", "confluence"]);
    }
}
