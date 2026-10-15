-- World of Kartcraft's courier deliveries, for the world database (mangos): the token they pay,
-- the package they carry, and the routes between the neutral towns' people, each a repeatable
-- quest. Kartcraft.cpp keeps the routes' par times and offers three of them a day (DELIVERIES);
-- its quest ids, item ids and NPCs are these. vmangos-deploy runs this from
-- storage/database/custom-sql/ on every database start, so it replaces its own rows, and only
-- those.

START TRANSACTION;

DELETE FROM `item_template` WHERE `entry` IN (190100, 190101);
DELETE FROM `quest_template` WHERE `entry` BETWEEN 190200 AND 190299;
DELETE FROM `creature_questrelation` WHERE `quest` BETWEEN 190200 AND 190299;
DELETE FROM `creature_involvedrelation` WHERE `quest` BETWEEN 190200 AND 190299;

-- The token (the Darkmoon Faire ticket's look), bound to who earns it; what it buys is to come.
-- The package (the sealed crate's look), a quest item: one at a time, gone with its quest.
INSERT INTO `item_template` (`entry`, `patch`, `class`, `subclass`, `name`, `description`,
  `display_id`, `quality`, `flags`, `buy_price`, `sell_price`, `bonding`, `max_count`, `stackable`) VALUES
(190100, 0, 15, 0, 'Kartcraft Token', 'Paid for a courier delivery. Keep them: what they buy is on its way.',
  31745, 2, 0, 0, 0, 1, 0, 1000),
(190101, 0, 12, 0, 'Courier''s Package', 'By kart only: no flights, no hearthstones, no portals.',
  8928, 1, 0, 0, 0, 4, 1, 1);

-- The routes: from a town's quest giver (creature_questrelation) to another's
-- (creature_involvedrelation), the package handed over on accepting and taken back on
-- delivering, ten tokens paid. Repeatable (SpecialFlags 1): the server allows one a day each.
INSERT INTO `quest_template` (`entry`, `patch`, `Method`, `ZoneOrSort`, `MinLevel`, `QuestLevel`,
  `SpecialFlags`, `SrcItemId`, `SrcItemCount`, `Title`, `Details`, `Objectives`, `OfferRewardText`,
  `RequestItemsText`, `ReqItemId1`, `ReqItemCount1`, `RewItemId1`, `RewItemCount1`) VALUES
(190200, 0, 2, 17, 1, 60, 1, 190101, 1, 'Express Delivery: Gadgetzan',
  'This crate has to reach Marin Noggenfogger in Gadgetzan, and it goes by kart: no flight paths, no hearthstones, no portals while you carry it.$B$BGet it there within 5 minutes and there''s a bonus in it for you.',
  'Deliver the Courier''s Package to Marin Noggenfogger in Gadgetzan.',
  'My crate! Here''s your pay, courier.', 'You have something for me?', 190101, 1, 190100, 10),
(190201, 0, 2, 440, 1, 60, 1, 190101, 1, 'Express Delivery: Ratchet',
  'Wharfmaster Dizzywig in Ratchet is waiting on this crate. By kart, mind you: no flight paths, no hearthstones, no portals while you carry it.$B$BMake Ratchet within 5 minutes for a bonus.',
  'Deliver the Courier''s Package to Wharfmaster Dizzywig in Ratchet.',
  'Finally! Here''s your pay, courier.', 'Is that my crate?', 190101, 1, 190100, 10),
(190202, 0, 2, 440, 1, 60, 1, 190101, 1, 'Express Delivery: Cenarion Hold',
  'Take this crate to Calandrath at Cenarion Hold, in Silithus. By kart only: no flight paths, no hearthstones, no portals while you carry it.$B$BThere within 4 minutes earns a bonus.',
  'Deliver the Courier''s Package to Calandrath at Cenarion Hold.',
  'Just in time. Here''s your pay, courier.', 'Is that for me?', 190101, 1, 190100, 10),
(190203, 0, 2, 1377, 1, 60, 1, 190101, 1, 'Express Delivery: Gadgetzan',
  'Marin Noggenfogger in Gadgetzan ordered this crate. By kart only: no flight paths, no hearthstones, no portals while you carry it.$B$BReach Gadgetzan within 4 minutes for a bonus.',
  'Deliver the Courier''s Package to Marin Noggenfogger in Gadgetzan.',
  'My crate! Here''s your pay, courier.', 'You have something for me?', 190101, 1, 190100, 10),
(190204, 0, 2, 17, 1, 60, 1, 190101, 1, 'Express Delivery: Everlook',
  'This one goes all the way north, to Innkeeper Vizzie in Everlook. By kart only: no flight paths, no hearthstones, no portals while you carry it.$B$BEverlook within 7 minutes earns a bonus.',
  'Deliver the Courier''s Package to Innkeeper Vizzie in Everlook.',
  'All the way from Ratchet! Here''s your pay, courier.', 'Did you bring my crate?', 190101, 1, 190100, 10),
(190205, 0, 2, 33, 1, 60, 1, 190101, 1, 'Express Delivery: Thorium Point',
  'Hansel Heavyhands at Thorium Point, up in the Searing Gorge, needs this crate. By kart only: no flight paths, no hearthstones, no portals while you carry it.$B$BThere within 7 minutes earns a bonus.',
  'Deliver the Courier''s Package to Hansel Heavyhands at Thorium Point.',
  'Good, good. Here''s your pay, courier.', 'That my crate?', 190101, 1, 190100, 10),
(190206, 0, 2, 51, 1, 60, 1, 190101, 1, 'Express Delivery: Light''s Hope Chapel',
  'Caretaker Alen at Light''s Hope Chapel, in the Eastern Plaguelands, is expecting this crate. A long road, by kart only: no flight paths, no hearthstones, no portals while you carry it.$B$BThere within 10 minutes earns a bonus.',
  'Deliver the Courier''s Package to Caretaker Alen at Light''s Hope Chapel.',
  'The Light keeps you, courier. Here''s your pay.', 'You have a delivery for the chapel?', 190101, 1, 190100, 10);

INSERT INTO `creature_questrelation` (`id`, `quest`) VALUES
(3453, 190200), (7564, 190201), (7564, 190202), (15174, 190203), (3453, 190204), (2496, 190205), (14627, 190206);

INSERT INTO `creature_involvedrelation` (`id`, `quest`) VALUES
(7564, 190200), (3453, 190201), (15174, 190202), (7564, 190203), (11118, 190204), (14627, 190205), (11038, 190206);

COMMIT;
