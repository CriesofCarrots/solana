# populate this on the stable branch
splTokenCliVersion=

maybeSplTokenCliVersionArg=
if [[ -n "$splTokenCliVersion" ]]; then
    maybeSplTokenCliVersionArg="--version $splTokenCliVersion"
fi
