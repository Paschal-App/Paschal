-- 0005_pricing_tiers.sql — Four-tier pricing ladder.
--
-- The market scan in market-scan-and-pricing.md repositions the product
-- against the wider category. New plan IDs added:
--
--   draft_v1                  — free tier (100 MB, 1y retention)
--   estate_monthly_v2         — $3/mo (1 GB, 3y retention)         ← new default
--   estate_annual_v2          — $30/yr (1 GB, 3y retention)        ← new default annual
--   estate_plus_monthly_v1    — $8/mo (10 GB, 10y retention)
--   estate_plus_annual_v1     — $80/yr (10 GB, 10y retention)
--   legacy_monthly_v1         — $20/mo (100 GB, 25y retention)
--   legacy_annual_v1          — $200/yr (100 GB, 25y retention)
--
-- The existing estate_monthly_v1 and estate_annual_v1 ($2/mo, $20/yr) stay
-- valid. Existing principals are grandfathered for at least 12 months. A
-- separate (manual) migration will roll them onto the new v2 IDs once the
-- 12-month window elapses.

ALTER TABLE subscription DROP CONSTRAINT IF EXISTS subscription_plan_id_check;
ALTER TABLE subscription ADD CONSTRAINT subscription_plan_id_check
    CHECK (plan_id IN (
        'draft_v1',
        'estate_monthly_v1',
        'estate_annual_v1',
        'estate_monthly_v2',
        'estate_annual_v2',
        'estate_plus_monthly_v1',
        'estate_plus_annual_v1',
        'legacy_monthly_v1',
        'legacy_annual_v1'
    ));

-- The legacy_estate_v1 plans had a hard-coded $2 / $20 price and 3-year
-- retention. Mark the in-flight ones explicitly so we can find them later
-- without ambiguous queries. (No data changes — just an index for the
-- eventual reprice migration.)
CREATE INDEX IF NOT EXISTS subscription_grandfathered_idx
    ON subscription(plan_id)
    WHERE plan_id IN ('estate_monthly_v1', 'estate_annual_v1');
