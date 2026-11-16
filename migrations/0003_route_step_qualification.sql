-- The tools a route step is qualified to run on, by id. The dispatcher checks
-- this list together with the tool's own qualification matrix.
ALTER TABLE route_step
    ADD COLUMN IF NOT EXISTS equipment_qualification JSONB NOT NULL DEFAULT '[]'::jsonb;
