# Recipes for blocks still unverified (from the static analyses, 2026-10-04)

All --mode runs: a card with Melee's save, MONKEY_HOLD none@0-99,start@100-103 to leave the title.

## Multi-man / attract demo (gm_181A)
- gm_80182174: --mode 22 (100-man) and --mode 24 (15-minute): pick a fighter and start.
- fn_80181E18+0x94, fn_80182B5C+0x208/+0x214: 3-/15-minute survival to the end (LOCKSTEP_UNLIMITED=fn_80181E18).
- fn_80182F40 (7): leave the title idle until the attract demo; LOCKSTEP_UNLIMITED=fn_80182F40 (frames 0x280, 0x370).

## Main library (gmmain_lib)
- gmMainLib_8015DBF4 (53): NAMES=3 and SAVE_POKE value 2 (size 1) at gmm_x0 0x520,0x526,0x52c,0x534,0x586, 0x590+0x140k+0x72+0x24i (k 0..12, i 0..2), 0x1860,0x1861,0x1863; MENU=2,4 Name Entry, delete name 0.
- D508/D5DC/D640: PROBES lines (no arguments) after SAVE_POKE all 25 target bits.
- FA34+0x10c/+0xe4: a save with deflicker off (gmm 0x1CC5=0), then --mode 27 Yes.

## Name entry (mnname, mnnamenew)
- NAMES=30: grid scroll right/left, R/L, A on a wrapped column, delete prompt scrolled.
- SortNames: NAMES=27 then Sort, delete a name below 26. CompareNameStrings: NAMES=2 + SAVE_POKE=0x3334:8:0x8260826081400000.
- mnnamenew: PickAutoName (random key 0x37 ~50 times, LOCKSTEP_UNLIMITED=PickAutoName); spaces in chars 3-4; Backspace on empty.
- NAMES raw bytes with % formats ("%lld","%hhx","%#.3s","%.500f", %n last alone) for MSL printf (~56).

## Classic intro (gm_1832), game over (gm_19EF)
- CLASSIC_STAGE=4 (kind 2), 7 (kind 4, ~30 blocks; also GAME_LANGUAGE=jp), 8 (race), 9 (metal); jp with 2 and 5.
- gm_19EF: SAVE_POKE coins=0, lose Classic, Continue (error); coins=9999 continue countdown.

## Tournament (gmtou_0/1/2, gmtoulib)
- --mode 1b CPU_PLAYERS=1 long runs at 6/12/24/48 entrants; L+R on the match-up screen (random fighter); hold the setup cursor on options 0x1B-0x1E 100+ fields (LOCKSTEP_UNLIMITED=fn_8019BF8C); SAVE_POKE handicap auto.

## Results (gmresult), events (mnevent), records (mndiagram2/3)
- Pause then L+R+A+Start (cancelled match); team stock ties.
- mnEvent_8024CE74: SAVE_POKE=0x1A68:8:<bits> for unlocked event counts 6-9,10-15,16-21,22-26,27-49,>=50 (and x7C.b5 at 0x1FA8+0xAC*i bit 0x04), then the Event list.
- mndiagram2 GetAggregatedFighterRank / mndiagram3: records R -> Rankings, X (name mode), NAMES>=1 RECORDS=seed, scroll.
- mnstagesw 80236CBC: open Random Stage Select once.

## Audio, effects, JPEG, pads (libs)
- lbaudio_ax: Sound Test SFX of banks idx%5==4; Kirby-team sound unloaded; >16 KO sounds in 10 frames; SFX bank load in flight at scene exit (DVD_RETRY).
- eflib: four Ice Climbers shielding together (MONKEY_HOLD r on p1..p4).
- hsd_3B34: Camera Mode snapshot of a busy scene with GX_RENDER.
- hsd_803931A4+0x74/+0x98: DBLEVEL with R held at boot.

## Stages / fighters (earlier batches; see report text in the levers notes)
- Great Bay turtle/moon, Big Blue lanes: long matches with LOCKSTEP_UNLIMITED on the stage procs (jobs-followup.txt).
- Shrine Route: Adventure Underground Maze (ADVENTURE_STAGE) or debug VS stage 63 with a stick-scripted walk to a pedestal.
- Kirby copies of Link (MATCH_FIGHTERS=4,6 / 4,21, --matches 1210) and Jigglypuff; Ness up-B self-hit (MATCH_FIGHTERS=11 --matches 20, FD=32).
- ftCo_0A01: MATCH_CPU_KIND 7..29 with items; ftCo_Damage: grabs hit by third players, stamina, hammer.
- DVD: DVD_FATAL / DVD_COVER mid-read with LOCKSTEP_UNLIMITED on dvd error functions.

## Stages batch 2 (grvenom, grrcruise, grcorneria, grzebes, groldkongo, grinishie1) and fighters
- Debug VS --mode e, MATCH_STAGES=<stkind>, long MATCH_TIME, LOCKSTEP_UNLIMITED on the stage procs (stageGObj1/2_* by address, e.g. 0x8020F888).
- Venom 22: arwing spawn needs 3+ background cycles (~10000 fields each), LOCKSTEP_UNLIMITED=grVenom_8020362C; Star Fox talk: Fox (2) or Falco (20) taps down for one field standing (MONKEY_HOLD=down@300-300) then idle ~900 fields, UNLIMITED=grVenom_80204CEC.
- Corneria 71 (multi-arwing), <4 players for the empty-slot shot; Fox talk interrupted by a hit.
- Rainbow Cruise 11: UNLIMITED=grRCruise_80200154,8020071C,80200C04,80201588 (platforms falling off camera, vanish state 3).
- Brinstar 6: ~13000 fields, UNLIMITED=grZebes_801D99E0 (acid schedule wraps), 801D95B8.
- Kongo Jungle 64 (30): >2 minutes, UNLIMITED=stageGObj1 at its address; a fighter falling into the barrel.
- Master Hand 26 / Crazy Hand 30 as humans: needs a runner option making a boss human (choose() forces CPU); Master Hand reads port 3's pad, Crazy Hand port 4's, button+D-pad pairs (L+up slap, ... Y+up combo). Both hands: MATCH_FIGHTERS=26,30,<playable>.
- Yoshi 17 egg roll (side-B) into walls both sides, turning mid-roll, off ledges, landing; LOCKSTEP_MUTATE for its abs/NaN arms.
- Link/Young Link holding an item uses ground neutral B (SpecialNEnd_Anim+0x9c).
- Stages batch 3 and gm/mn extras: per-unit reach groups and recipes in the scratchpad's gr3/notes/ (copied to local/lockstep/notes-gr3/).
