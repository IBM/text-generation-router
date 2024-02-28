#!/usr/bin/env bash

# Returns a space separated list of container image tags to be pushed to the
# registry
# If a repository is supplied as the first arg, (e.g. quay.io/foo/bar") then the tags will be fully qualified

tags_to_push=""

if [[ -n $1 ]]; then
  repo_bit="${1}:"
else
  repo_bit=""
fi

# if running locally, i.e. CI is unset
if [[ -z "${CI+x}" ]];
then
  commit="$(git rev-parse --short HEAD)"
  branch="$(git rev-parse --abbrev-ref HEAD)"

  tags_to_push+="${repo_bit}${commit}"
  tags_to_push+=" ${repo_bit}${branch}"
  tags_to_push+=" ${repo_bit}${branch}.${commit}"
else
  # In CI, pull info from github env vars
  commit="${GITHUB_SHA:0:7}"
  build_ref="${GITHUB_REF_NAME}"

  tags_to_push+="${repo_bit}${commit}"
  if [[ ! ${build_ref} =~ "merge" ]];
  then
    tags_to_push+=" ${repo_bit}${build_ref}"
    tags_to_push+=" ${repo_bit}${build_ref}.${commit}"
  fi
fi

echo "${tags_to_push}"
