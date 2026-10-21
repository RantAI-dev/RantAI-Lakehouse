"""Google analytics source settings and constants"""

START_DATE = "2015-08-14"

# SRC-3 V-GA4.1 (local change 2): days re-read before the last day loaded. Google says
# "Data processing can take 24-48 hours. During that time, data in your reports may
# change." (https://support.google.com/analytics/answer/11198161), so a day is final only
# two days later; three days cover the 48 hours and the edge of the day.
LOOKBACK_DAYS = 3
