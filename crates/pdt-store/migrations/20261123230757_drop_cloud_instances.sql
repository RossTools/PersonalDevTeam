-- Cloud instances were removed (PDT-735): drop the tables 20261123154821_cloud_instances added. Joined instances
-- (remote_instances) keep their rows.
DROP TABLE IF EXISTS cloud_usage;
DROP TABLE IF EXISTS cloud_instances;
