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

.PHONY: docker-image test-linux bench-linux test-apfs bench-apfs demo-gif shell-linux check test

docker-image:
	docker build -t $(DOCKER_IMAGE) docker

# Run the full test suite (including OverlayFS integration tests) inside Linux.
# --privileged is required for mount(2); tests never touch the host filesystem.
test-linux: docker-image
	$(DOCKER_RUN) cargo test

# The undo performance benchmark (< 100ms on a ~10k-file tree).
bench-linux: docker-image
	$(DOCKER_RUN) cargo test --release bench_undo -- --ignored --nocapture

# APFS backend tests: destructive but triple-gated (self-created tempdirs,
# per-test XDG_STATE_HOME override, and this explicit flag). macOS host only.
test-apfs:
	OOPS_TEST_DESTRUCTIVE=1 cargo test

# The APFS undo benchmark (< 100ms on a ~10k-file tree) + setup cost report.
bench-apfs:
	OOPS_TEST_DESTRUCTIVE=1 cargo test --release bench_undo_apfs -- --ignored --nocapture

# Re-render demo/demo.gif from demo/demo.tape (spec: must stay <= 3 MB).
demo-gif: docker-image
	docker build -t oops-demo -f docker/demo.Dockerfile docker
	docker run --rm --privileged \
		--tmpfs /root/.local/state/oops \
		-v $(PWD):/oops \
		-v oops-cargo-registry:/usr/local/cargo/registry \
		-v oops-target-linux:/oops/target \
		oops-demo bash -c '\
		cargo build --release \
		&& install -m755 target/release/oops /usr/local/bin/oops \
		&& mkdir -p /root/demo \
		&& vhs demo/demo.tape \
		&& size=$$(stat -c%s demo/demo.gif) \
		&& echo "demo.gif: $$size bytes" \
		&& test $$size -le 3145728'

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
