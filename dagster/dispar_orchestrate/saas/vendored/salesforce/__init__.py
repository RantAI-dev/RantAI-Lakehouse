"""Source for Salesforce, read through the connector's `SaasSession` (SRC-3 local change 1;
upstream used the simple_salesforce package).

Imported resources are: account, campaign, contact, lead, opportunity, pricebook_2, pricebook_entry, product_2, user and user_role

Salesforce api docs: https://developer.salesforce.com/docs/apis
"""

from dlt.sources import DltResource
from dlt.sources import incremental

from typing import Any, Iterable

import dlt
from dlt.common.typing import TDataItem

from .helpers.records import get_records


@dlt.source(name="salesforce")
def salesforce_source(session: Any) -> Iterable[DltResource]:
    # SRC-3 V-Salesforce.1 (local change 1): the `SaasSession` of the connector
    # replaces upstream's credentials and `simple_salesforce` client.
    client = session

    # define resources
    @dlt.resource(write_disposition="replace")
    def sf_user() -> Iterable[TDataItem]:
        yield get_records(client, "User")

    @dlt.resource(write_disposition="replace")
    def user_role() -> Iterable[TDataItem]:
        yield get_records(client, "UserRole")

    @dlt.resource(write_disposition="merge")
    def opportunity(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(
            client, "Opportunity", last_timestamp.last_value, "SystemModstamp"
        )

    @dlt.resource(write_disposition="merge")
    def opportunity_line_item(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(
            client, "OpportunityLineItem", last_timestamp.last_value, "SystemModstamp"
        )

    @dlt.resource(write_disposition="merge")
    def opportunity_contact_role(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(
            client,
            "OpportunityContactRole",
            last_timestamp.last_value,
            "SystemModstamp",
        )

    @dlt.resource(write_disposition="merge")
    def account(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "LastModifiedDate", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(
            client, "Account", last_timestamp.last_value, "LastModifiedDate"
        )

    @dlt.resource(write_disposition="replace")
    def contact() -> Iterable[TDataItem]:
        yield get_records(client, "Contact")

    @dlt.resource(write_disposition="replace")
    def lead() -> Iterable[TDataItem]:
        yield get_records(client, "Lead")

    @dlt.resource(write_disposition="replace")
    def campaign() -> Iterable[TDataItem]:
        yield get_records(client, "Campaign")

    @dlt.resource(write_disposition="merge")
    def campaign_member(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(
            client, "CampaignMember", last_timestamp.last_value, "SystemModstamp"
        )

    @dlt.resource(write_disposition="replace")
    def product_2() -> Iterable[TDataItem]:
        yield get_records(client, "Product2")

    @dlt.resource(write_disposition="replace")
    def pricebook_2() -> Iterable[TDataItem]:
        yield get_records(client, "Pricebook2")

    @dlt.resource(write_disposition="replace")
    def pricebook_entry() -> Iterable[TDataItem]:
        yield get_records(client, "PricebookEntry")

    @dlt.resource(write_disposition="merge")
    def task(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(client, "Task", last_timestamp.last_value, "SystemModstamp")

    @dlt.resource(write_disposition="merge")
    def event(
        last_timestamp: incremental[str] = dlt.sources.incremental(
            "SystemModstamp", initial_value=None
        )
    ) -> Iterable[TDataItem]:
        yield get_records(client, "Event", last_timestamp.last_value, "SystemModstamp")

    return (
        sf_user,
        user_role,
        opportunity,
        opportunity_line_item,
        opportunity_contact_role,
        account,
        contact,
        lead,
        campaign,
        campaign_member,
        product_2,
        pricebook_2,
        pricebook_entry,
        task,
        event,
    )
