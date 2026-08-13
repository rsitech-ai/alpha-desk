BEGIN;

CREATE TABLE entity_annotations (
    evidence_id text PRIMARY KEY
        CHECK (evidence_id <> '' AND evidence_id = btrim(evidence_id)),
    left_node text NOT NULL
        CHECK (left_node <> '' AND left_node = btrim(left_node)),
    right_node text NOT NULL
        CHECK (right_node <> '' AND right_node = btrim(right_node)),
    reviewer text NOT NULL
        CHECK (reviewer <> '' AND reviewer = btrim(reviewer)),
    policy_version text NOT NULL
        CHECK (policy_version <> '' AND policy_version = btrim(policy_version)),
    approved_at timestamptz NOT NULL,
    CHECK (left_node <> right_node)
);

COMMIT;
