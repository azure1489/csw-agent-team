-- +goose Up
-- r45 暴露：收集员为核实线索去读官方页面（佐证），这些页面被计进 reviewed，
-- 但佐证不是候选、不该登记成条目，于是判据二把「已审 18 > 登记 14」误判成「审过没落状态」。
-- 佐证单独计数：判据二改用 reviewed - corroborated 对账。
ALTER TABLE intake_sweeps ADD COLUMN corroborated INTEGER NOT NULL DEFAULT 0;

-- +goose Down
ALTER TABLE intake_sweeps DROP COLUMN corroborated;
