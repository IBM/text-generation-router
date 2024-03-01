SHELL := /bin/bash

# ghcr.io/ibm/text-generation-router:${tag}
docker_host := ghcr.io
docker_registry := $(docker_host)/ibm

server_image_name := text-generation-router
server_image_target := router-release
server_image_repo := $(docker_registry)/$(server_image_name)

has_gawk := $(shell gawk --version 2>/dev/null)
.PHONY: help
help: ## Display this help.
ifdef has_gawk
	@gawk -f ./scripts/makefile.help.awk $(MAKEFILE_LIST)
else
	@awk 'BEGIN{FS=":.*##"; printf("\nUsage:\n  make \033[36m<target>\033[0m\n\n")} /^[-a-zA-Z_0-9\\.]+:.*?##/ {t=$$1; if(!(t in p)){p[t]; printf("\033[36m%-15s\033[0m %s\n", t, $$2)}}' $(MAKEFILE_LIST)
	@echo
	@echo "NOTE: Help output with headers requires GNU extensions to awk. Please install gawk for the best experience."
endif

##@ Container Build Tasks

.PHONY: build-router
build-router: ##
	DOCKER_BUILDKIT=1 docker build --target $(server_image_target) \
	--progress plain \
	--build-arg BUILDKIT_INLINE_CACHE=1 \
	--tag "$(server_image_name)" .
	docker images
