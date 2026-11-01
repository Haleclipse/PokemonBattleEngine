import {Teams, TeamValidator} from '@pkmn/sim';

// A neutral scenario set as Showdown's `Teams.pack` takes it, shared by the recorder
// (`showdown.mjs`) and the corpus player (`corpus.mjs`) so that both build the same battle.
// Scenario sets carry a `label`; Showdown wants a `name`. A Gen 3+ set without an ability has its
// species' first one, as `dex::build_pokemon` gives it (Showdown would leave it without).
export const packable = (battle, set) => ({
  name: set.label ?? set.name ?? set.species,
  ...set,
  ...(battle.gen >= 3 && !set.ability ? {ability: battle.dex.species.get(set.species).abilities['0']} : {}),
});

// A side's team as the battle receives it: a custom game's as written; a real format's as its
// validator leaves it (the server's path, DESIGN.md §45.5: levels adjusted, EVs and IVs filled
// in …), refused when the validator finds a problem.
export function battleTeam(battle, formatid, sets) {
  const team = sets.map(set => packable(battle, set));
  if (!/customgame$/.test(formatid)) {
    const problems = new TeamValidator(formatid).validateTeam(team);
    if (problems) throw new Error(`${formatid}: ${problems.join('; ')}`);
  }
  return Teams.pack(team);
}
