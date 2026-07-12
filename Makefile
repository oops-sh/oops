# Development entry points. The host is macOS; everything that touches
# OverlayFS must run inside the Linux container (test-linux).

DOCKER_IMAGE := oops-dev
# --tmpfs: the container root is overlay2, and OverlayFS refuses an upperdir
# that itself lives on overlay — so the oops state dir must be tmpfs in tests.
DOCKER_RUN   := docker run --rm --privileged \
	--tmpfs /root/.local/state/oops \
	-v $(PWD):/oops \
	-v oops-cargo-registry:/usr/local/cargo/registry \
	-v oops-target-linux:/oops/target \
	$(DOCKER_IMAGE)

.PHONY: docker-image test-linux shell-linux check test

docker-image:
	docker build -t $(DOCKER_IMAGE) docker

# Run the full test suite (including OverlayFS integration tests) inside Linux.
# --privileged is required for mount(2); tests never touch the host filesystem.
test-linux: docker-image
	$(DOCKER_RUN) cargo test

# Interactive shell inside the Linux test environment.
shell-linux: docker-image
	docker run --rm -it --privileged \
		--tmpfs /root/.local/state/oops \
		-v $(PWD):/oops \
		-v oops-cargo-registry:/usr/local/cargo/registry \
		-v oops-target-linux:/oops/target \
		$(DOCKER_IMAGE) bash

# Fast host-side checks (no OverlayFS): compile + unit tests that are OS-independent.
check:
	cargo check

test:
	cargo test
