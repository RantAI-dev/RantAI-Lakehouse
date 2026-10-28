/**
 * The business-app (SaaS) vendors of `SRC-3`, as the console needs them: which
 * connector type is which vendor, what the account field asks for, which
 * sign-ins the vendor offers, and which credential fields an integration
 * credential fills (decision 3).
 *
 * `VENDORS` is a literal copy of `ops/fixtures/saas_vendors.json`, the file the
 * Rust table (`lakehouse-store/src/saas_vendors.rs`) and the orchestrator's
 * registry are also compared with; `saas-vendors.test.ts` compares this copy.
 * It is a copy and not an import because the fixture sits outside `src/`. The
 * console never builds an address from it: the account is sent as typed and
 * the API validates it against the same table (AD2).
 */

export type SaasAccountRule =
  | { kind: "label" | "digits" | "account_id" | "dataverse_host" | "label_tenant" | "dataverse_host_tenant" }
  | { kind: "host_tenant"; suffixes: readonly string[] }

export type SaasVendor = {
  id: string
  label: string
  /** The `connector_type.name` this vendor is listed under. */
  connectorType: string
  account: SaasAccountRule
  /** What the account field asks for. */
  accountLabel: string
  apiBase: string
  oauth: {
    authorizeUrl: string
    tokenUrl: string
    revocationUrl: string | null
    scopes: readonly string[]
    clientAuth: "post" | "basic" | "post_json"
    /** The API's base address after a browser sign-in, when it is not `apiBase` (Atlassian's gateway). */
    apiBase?: string
  }
  /** The integration credential, when the vendor offers one (Salesforce does not: decision 4). */
  integration: {
    secretKey: string
    secretFields: readonly string[]
    /** The scope of a client-credentials request, when the vendor needs one (Microsoft's `/.default`). */
    scope?: string
  } | null
  whoami: { method: string; path: string } | null
}

export const SAAS_VENDORS: readonly SaasVendor[] = [
  {
    "id": "salesforce",
    "label": "Salesforce",
    "connectorType": "Salesforce",
    "account": {
      "kind": "label"
    },
    "accountLabel": "My Domain name (the part before .my.salesforce.com)",
    "apiBase": "https://{account}.my.salesforce.com",
    "oauth": {
      "authorizeUrl": "https://{account}.my.salesforce.com/services/oauth2/authorize",
      "tokenUrl": "https://{account}.my.salesforce.com/services/oauth2/token",
      "revocationUrl": "https://{account}.my.salesforce.com/services/oauth2/revoke",
      "scopes": [
        "api",
        "refresh_token"
      ],
      "clientAuth": "post"
    },
    "integration": null,
    "whoami": {
      "method": "GET",
      "path": "/services/data/v62.0/limits"
    }
  },
  {
    "id": "servicenow",
    "label": "ServiceNow",
    "connectorType": "ServiceNow",
    "account": {
      "kind": "label"
    },
    "accountLabel": "Instance name (the part before .service-now.com)",
    "apiBase": "https://{account}.service-now.com",
    "oauth": {
      "authorizeUrl": "https://{account}.service-now.com/oauth_auth.do",
      "tokenUrl": "https://{account}.service-now.com/oauth_token.do",
      "revocationUrl": null,
      "scopes": [],
      "clientAuth": "post"
    },
    "integration": {
      "secretKey": "integration:servicenow",
      "secretFields": [
        "clientId",
        "clientSecret"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/api/now/table/sys_user?sysparm_limit=1&sysparm_fields=sys_id"
    }
  },
  {
    "id": "workday",
    "label": "Workday Reports",
    "connectorType": "Workday Reports",
    "account": {
      "kind": "host_tenant",
      "suffixes": [
        ".workday.com",
        ".myworkday.com"
      ]
    },
    "accountLabel": "Workday host and tenant, as host/tenant",
    "apiBase": "https://{host}",
    "oauth": {
      "authorizeUrl": "https://{host}/{tenant}/authorize",
      "tokenUrl": "https://{host}/ccx/oauth2/{tenant}/token",
      "revocationUrl": null,
      "scopes": [],
      "clientAuth": "basic"
    },
    "integration": {
      "secretKey": "integration:workday",
      "secretFields": [
        "username",
        "password"
      ]
    },
    "whoami": null
  },
  {
    "id": "ga4",
    "label": "Google Analytics 4",
    "connectorType": "Google Analytics 4",
    "account": {
      "kind": "digits"
    },
    "accountLabel": "GA4 property id (digits)",
    "apiBase": "https://analyticsdata.googleapis.com",
    "oauth": {
      "authorizeUrl": "https://accounts.google.com/o/oauth2/v2/auth?access_type=offline&prompt=consent",
      "tokenUrl": "https://oauth2.googleapis.com/token",
      "revocationUrl": "https://oauth2.googleapis.com/revoke",
      "scopes": [
        "https://www.googleapis.com/auth/analytics.readonly"
      ],
      "clientAuth": "post"
    },
    "integration": {
      "secretKey": "integration:ga4",
      "secretFields": [
        "serviceAccountJson"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/v1beta/properties/{account}/metadata"
    }
  },
  {
    "id": "hubspot",
    "label": "HubSpot",
    "connectorType": "HubSpot",
    "account": {
      "kind": "digits"
    },
    "accountLabel": "HubSpot account id (digits)",
    "apiBase": "https://api.hubapi.com",
    "oauth": {
      "authorizeUrl": "https://app.hubspot.com/oauth/authorize",
      "tokenUrl": "https://api.hubapi.com/oauth/v1/token",
      "revocationUrl": null,
      "scopes": [
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
        "oauth"
      ],
      "clientAuth": "post"
    },
    "integration": {
      "secretKey": "integration:hubspot",
      "secretFields": [
        "token"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/account-info/v3/details"
    }
  },
  {
    "id": "zendesk",
    "label": "Zendesk",
    "connectorType": "Zendesk",
    "account": {
      "kind": "label"
    },
    "accountLabel": "Subdomain (the part before .zendesk.com)",
    "apiBase": "https://{account}.zendesk.com",
    "oauth": {
      "authorizeUrl": "https://{account}.zendesk.com/oauth/authorizations/new",
      "tokenUrl": "https://{account}.zendesk.com/oauth/tokens",
      "revocationUrl": null,
      "scopes": [
        "read"
      ],
      "clientAuth": "post_json"
    },
    "integration": {
      "secretKey": "integration:zendesk",
      "secretFields": [
        "email",
        "apiToken"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/api/v2/users/me.json"
    }
  },
  {
    "id": "confluence",
    "label": "Confluence",
    "connectorType": "Confluence",
    "account": {
      "kind": "label"
    },
    "accountLabel": "Site name (the part before .atlassian.net)",
    "apiBase": "https://{account}.atlassian.net",
    "oauth": {
      "authorizeUrl": "https://auth.atlassian.com/authorize?audience=api.atlassian.com&prompt=consent",
      "tokenUrl": "https://auth.atlassian.com/oauth/token",
      "revocationUrl": null,
      "scopes": [
        "read:page:confluence",
        "read:space:confluence",
        "read:comment:confluence",
        "offline_access"
      ],
      "clientAuth": "post_json",
      "apiBase": "https://api.atlassian.com/ex/confluence/{cloudId}"
    },
    "integration": {
      "secretKey": "integration:confluence",
      "secretFields": [
        "email",
        "apiToken"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/wiki/api/v2/spaces?limit=1"
    }
  },
  {
    "id": "dynamics365",
    "label": "Dynamics 365",
    "connectorType": "Dynamics 365",
    "account": {
      "kind": "dataverse_host_tenant"
    },
    "accountLabel": "Environment address and tenant, as org.crm4.dynamics.com/tenant-id-or-domain",
    "apiBase": "https://{account}",
    "oauth": {
      "authorizeUrl": "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
      "tokenUrl": "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
      "revocationUrl": null,
      "scopes": [
        "https://{account}/user_impersonation",
        "offline_access"
      ],
      "clientAuth": "post"
    },
    "integration": {
      "secretKey": "integration:dynamics365",
      "secretFields": [
        "clientId",
        "clientSecret"
      ],
      "scope": "https://{account}/.default"
    },
    "whoami": {
      "method": "GET",
      "path": "/api/data/v9.2/WhoAmI"
    }
  },
  {
    "id": "netsuite",
    "label": "NetSuite",
    "connectorType": "NetSuite",
    "account": {
      "kind": "account_id"
    },
    "accountLabel": "Account id (for example 1234567 or 1234567_SB1)",
    "apiBase": "https://{account}.suitetalk.api.netsuite.com",
    "oauth": {
      "authorizeUrl": "https://{account}.app.netsuite.com/app/login/oauth2/authorize.nl",
      "tokenUrl": "https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/token",
      "revocationUrl": "https://{account}.suitetalk.api.netsuite.com/services/rest/auth/oauth2/v1/revoke",
      "scopes": [
        "rest_webservices"
      ],
      "clientAuth": "basic"
    },
    "integration": {
      "secretKey": "integration:netsuite",
      "secretFields": [
        "clientId",
        "privateKey"
      ]
    },
    "whoami": {
      "method": "GET",
      "path": "/services/rest/record/v1/employee?limit=1"
    }
  },
  {
    "id": "sharepoint",
    "label": "SharePoint",
    "connectorType": "SharePoint",
    "account": {
      "kind": "label_tenant"
    },
    "accountLabel": "Tenant name and tenant, as contoso/tenant-id-or-domain (contoso is the part before .sharepoint.com)",
    "apiBase": "https://graph.microsoft.com",
    "oauth": {
      "authorizeUrl": "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/authorize",
      "tokenUrl": "https://login.microsoftonline.com/{tenant}/oauth2/v2.0/token",
      "revocationUrl": null,
      "scopes": [
        "https://graph.microsoft.com/Sites.Read.All",
        "offline_access"
      ],
      "clientAuth": "post"
    },
    "integration": {
      "secretKey": "integration:sharepoint",
      "secretFields": [
        "clientId",
        "clientSecret"
      ],
      "scope": "https://graph.microsoft.com/.default"
    },
    "whoami": {
      "method": "GET",
      "path": "/v1.0/sites/{account}.sharepoint.com"
    }
  }
] as const satisfies readonly SaasVendor[]

/** The vendor a connector type name stands for, or `undefined`. */
export function saasVendorForType(typeName: string | null | undefined): SaasVendor | undefined {
  return SAAS_VENDORS.find((vendor) => vendor.connectorType === typeName)
}

/** The vendor with this id (a `saas` dial's `vendor`), or `undefined`. */
export function saasVendorById(id: unknown): SaasVendor | undefined {
  return typeof id === "string" ? SAAS_VENDORS.find((vendor) => vendor.id === id) : undefined
}
