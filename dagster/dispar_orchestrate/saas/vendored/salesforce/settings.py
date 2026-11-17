"""Salesforce source settings and constants"""

# set to false to limit query to 100 records for testing
IS_PRODUCTION = True

# SRC-3 V-Salesforce.1 (local change 1): the REST API version every path carries.
# Salesforce keeps a version for years after it is superseded; moving this is a
# reviewed change, like the vendored code itself.
API_VERSION = "62.0"
