\set ON_ERROR_STOP on

CREATE SCHEMA IF NOT EXISTS alpha AUTHORIZATION alpha;

CREATE TABLE IF NOT EXISTS alpha.infrastructure_metadata (
    component text PRIMARY KEY,
    schema_version integer NOT NULL CHECK (schema_version > 0)
);

INSERT INTO alpha.infrastructure_metadata (component, schema_version)
VALUES ('stage-0-local-stack', 1)
ON CONFLICT (component) DO UPDATE
SET schema_version = EXCLUDED.schema_version;
