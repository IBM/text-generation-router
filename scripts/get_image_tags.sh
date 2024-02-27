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
  # In CI, pull info from TRAVIS_ env vars
  # we always have a commit
  commit="${TRAVIS_COMMIT:0:7}"

  # decide how to reference the build
  # If PR build, use the PR number
  # If tag build, use the tag
  # If branch build, use the branch
  if [[ "${TRAVIS_PULL_REQUEST}" == "false" ]];
  then
    # if it is a tag build, TRAVIS_BRANCH will be the tag
    build_ref="${TRAVIS_BRANCH}"
  else
    # TRAVIS_PULL_REQUEST contains the PR number
    build_ref="PR-${TRAVIS_PULL_REQUEST}"
  fi

  tags_to_push+="${commit}"
  tags_to_push+=" ${build_ref}"
  tags_to_push+=" ${build_ref}.${commit}"
fi

echo "${tags_to_push}"
