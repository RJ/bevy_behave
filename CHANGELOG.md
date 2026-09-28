## unreleased

* Ignore repeat status reports from task entities that already reported (awaiting despawn, or despawned), and stop `BehaveTimeout` reporting again while its entity awaits despawn. Since 0.5 despawns completed task entities a tick later, these repeat reports logged "Given node result for a non-spawned entity node?".

## 0.6.0

* Bevy 0.19 release
* `ego-tree` 0.11

## 0.5.0

* Bevy 0.18 release

## 0.3.0

* bevy 0.16 release