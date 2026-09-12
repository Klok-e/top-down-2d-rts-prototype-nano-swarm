# AI Battle verification

AI Battle is symmetric self-play between two swarms using the sole Strategic Controller. Standard and Flanks are the supported balanced layouts. Both sides use the same gameplay rules and shipped pacing, painting is disabled for the spectator, and the first Swarm Elimination supplies the actual winning `SwarmId` or a Draw.

The active launch controls are `--layout standard|flanks`, `--seed`, `--realtime`, and `--output-root`. There is no controller selection, side swapping, pacing selection, experiment mode, or automatic cutoff. An interrupted run remains `interrupted` with no invented outcome.

Battle statistics schema version 4 records factual effective damage, population, births, deaths, resources, structures, elimination time, simulation timing, and deterministic controller reviews, intent edits, work units, and latest explanation. Scored damage, lifetime credit caps, controller wall-clock samples, and observation wall-clock maxima are absent.

Repository-wide and real-process evidence for the sole-controller cutover is maintained in [Strategic Controller verification](strategic-controller.md). Historical verification for the earlier benchmark remains available through repository history and the archived [Strategic Controller research history](strategic-controller-research-history.md).
