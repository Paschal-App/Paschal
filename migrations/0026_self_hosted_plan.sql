-- 0026: allow the self-hosted edition's single plan id in the subscription
-- plan_id allowlist.
--
-- The open-source build has no billing, so `parse_plan_param` assigns every new
-- account `PlanId::SelfHosted` ("self_hosted") regardless of the plan chosen on
-- the sign-up screen. The allowlist CHECK from 0005 never included that id, so
-- the subscription INSERT in signup violated `subscription_plan_id_check` and
-- every account creation 500'd on a real deployment.
--
-- Expand-only (data safety rule 1): widens an allowlist CHECK. Safe to re-run.

ALTER TABLE subscription DROP CONSTRAINT IF EXISTS subscription_plan_id_check;
ALTER TABLE subscription ADD CONSTRAINT subscription_plan_id_check
    CHECK (plan_id IN (
        'self_hosted',
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
