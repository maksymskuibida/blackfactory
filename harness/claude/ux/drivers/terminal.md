# Driver: command-line interfaces

For screens whose `interface` is `terminal`. A plain shell is enough.

## Tools needed

- `sh` or `bash` (always present)
- `script` or `tee` (to keep a transcript)
- `tmux` only for a program that needs a live terminal (a full-screen program); without it such a step is unreachable

Check each with `command -v <tool>`. When one is missing, run `bf evidence unreachable <check> --reason "<tool> missing for terminal screen <S-id>"`. Never pass.

## Start the product

Build it as the brief says, then run the built program from the working copy by the path the brief gives. Use a throwaway home and working folder in the scratch folder (`HOME=<scratch>/home`), so a run never touches the real machine's files.

## Act

Run the command the journey step names with the arguments and input it gives, for example `printf 'text\n' | ./prog sub --flag`. Run it in a loop of one command per step. For a full-screen program: `tmux new-session -d -s ux -x 100 -y 30 './prog'`, send keys with `tmux send-keys -t ux '<keys>'`.

## Capture evidence

- The output and the exit code in one file: `./prog args > <scratch>/<id>.txt 2>&1; echo "exit $?" >> <scratch>/<id>.txt`
- For a full-screen program: `tmux capture-pane -p -t ux > <scratch>/<id>.txt`
- Check help text, error text and exit codes for each state the contract names. Run once with `NO_COLOR=1` to confirm meaning does not rest on colour.
- Attach one file per step with the step evidence command.

## Stop

`tmux kill-session -t ux` for a session you started. Remove nothing outside the scratch folder.
