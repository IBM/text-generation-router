#!/usr/bin/env bash

# Returns a space separated list of container image tags to be pushed to the
# registry

tags_to_push=""

# if running locally, i.e. CI is unset
if [[ -z "${CI+x}" ]];
then
  commit="$(git rev-parse --short HEAD)"
  branch="$(git rev-parse --abbrev-ref HEAD)"

  tags_to_push+="${commit}"
  tags_to_push+=" ${branch}"
  tags_to_push+=" ${branch}.${commit}"
else
  # In CI, pull info from github env vars
  commit="${GITHUB_SHA:0:7}"
  build_ref="${GITHUB_REF_NAME}"

  tags_to_push+="${commit}"
  tags_to_push+=" ${build_ref}"
  tags_to_push+=" ${build_ref}.${commit}"
fi

echo "${tags_to_push}"
