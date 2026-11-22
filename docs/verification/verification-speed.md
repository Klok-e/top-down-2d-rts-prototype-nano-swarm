# Screenshot and battle scheduling

In a seed-11 comparison, running the screenshot suite first and then running Standard and Flanks in parallel took **16m 56s**. Starting all three together took **19m 11s**. The screenshot suite passed all 42 cases in both runs and took 45s separately versus 136s during the battles.

Both Standard battles reached the 108,000-tick cap. Flanks ended by elimination at tick 78,901 with separate scheduling and tick 79,857 with overlap. The timings therefore include some variation in gameplay work.
