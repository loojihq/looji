-- A project's working schedule: the minutes since midnight its working day starts and ends, and the weekdays
-- it runs on as a mask (Monday is bit 0, Sunday is bit 6). The defaults are Monday to Friday, 09:00 to 17:00,
-- the same values as WorkingSchedule::default in the domain, and a test keeps them equal.
--
-- The CHECKs are the database's own rules, a second line behind the domain. The end must come after the start
-- and no later than the end of the day. The mask must name at least one weekday and nothing above Sunday.
ALTER TABLE projects ADD COLUMN working_start_min INTEGER NOT NULL DEFAULT 540
    CHECK (working_start_min >= 0 AND working_start_min < 1440);
ALTER TABLE projects ADD COLUMN working_end_min INTEGER NOT NULL DEFAULT 1020
    CHECK (working_end_min > working_start_min AND working_end_min <= 1440);
ALTER TABLE projects ADD COLUMN workdays_mask INTEGER NOT NULL DEFAULT 31
    CHECK (workdays_mask >= 1 AND workdays_mask <= 127);
