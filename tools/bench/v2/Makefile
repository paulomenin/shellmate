# Run the v2 model evaluation in a container (Docker or Podman).
#
#   make build-podman        build the image with podman      (build-docker: with docker)
#   make run-podman          evaluate the models in $(CONFIG) with podman
#   make run-docker          the same with docker
#   make run-podman-behavior   as run-podman, and ask before running each behavioral candidate (-docker too)
#   make run-podman-auto       as run-podman-behavior, but no questions: commands run unattended in the container
#                              (never one the product rates high risk; those stay incomplete) (-docker too)
#   make replay-podman       no model: replay the shipped reference cassette (plumbing check; -docker too)
#   make aggregate           merge every run in reports/ into reports/summary.md (tiers, categories, speed, cost)
#   make docs-page           the same as a documentation page: $(DOCS_PAGE), MACHINE="CPU, RAM, GPU" says what ran it
#   make docs-cases          write the page that describes every case, check and error ($(CASES_PAGE))
#   make check-podman        build the image and run all the tests, including the replay of both cassettes (-docker too)
#   make test                the unit tests (no container, no model)
#   make plan                validate $(CONFIG) and show the run matrix (no container, no model)
#
# Each run writes to reports/<timestamp>/ (git-ignored). Change the config with CONFIG=other.toml.
# A provider key goes in the config as api_key = "file:..." / "env:..." / "prompt:..." (see README.md).

CONFIG ?= models.toml
IMAGE  ?= shellmate-evals:v2
STAMP  := $(shell date +%Y%m%d-%H%M%S)
OUT    ?= reports/$(STAMP)
ROOT   := $(abspath ../../..)
PYTHON ?= python3

DOCS_PAGE ?= ../../../website/src/content/docs/docs/benchmarks/models-v2.md
MACHINE   ?=
CASES_PAGE ?= ../../../website/src/content/docs/docs/benchmarks/cases-v2.md

.PHONY: aggregate docs-page docs-cases check-docker check-podman help build-docker build-podman run-docker run-podman run-docker-behavior run-podman-behavior run-docker-auto run-podman-auto replay-docker replay-podman test plan

help:
	@sed -n '2,18p' Makefile

build-docker:
	docker build -f Dockerfile -t $(IMAGE) $(ROOT)

build-podman:
	podman build -f Dockerfile -t $(IMAGE) $(ROOT)

run-docker: check-config
	SHELLMATE_EVAL_ENGINE=docker $(PYTHON) evals.py --config $(CONFIG) --out $(OUT)

run-podman: check-config
	SHELLMATE_EVAL_ENGINE=podman $(PYTHON) evals.py --config $(CONFIG) --out $(OUT)

# --behavior asks for a typed "yes" at the terminal for each command, so these need a real terminal.
run-docker-behavior: check-config
	SHELLMATE_EVAL_ENGINE=docker $(PYTHON) evals.py --config $(CONFIG) --behavior --out $(OUT)

run-podman-behavior: check-config
	SHELLMATE_EVAL_ENGINE=podman $(PYTHON) evals.py --config $(CONFIG) --behavior --out $(OUT)

# Unattended: nobody reads the commands. They still run only in the networkless container, and a command the
# product rates high risk is not run. Use it for models you are willing to run commands from, in a container
# engine you keep updated.
run-docker-auto: check-config
	SHELLMATE_EVAL_ENGINE=docker $(PYTHON) evals.py --config $(CONFIG) --behavior --auto-approve --out $(OUT)

run-podman-auto: check-config
	SHELLMATE_EVAL_ENGINE=podman $(PYTHON) evals.py --config $(CONFIG) --behavior --auto-approve --out $(OUT)

replay-docker:
	SHELLMATE_EVAL_ENGINE=docker $(PYTHON) evals.py --config replay.toml --replay cassettes --out $(OUT)

replay-podman:
	SHELLMATE_EVAL_ENGINE=podman $(PYTHON) evals.py --config replay.toml --replay cassettes --out $(OUT)

# Every directory under reports/ that was a live run with the same suite. Replays are skipped.
aggregate:
	$(PYTHON) aggregate.py reports $(if $(MACHINE),--machine "$(MACHINE)") -o reports/summary.md

docs-page:
	$(PYTHON) aggregate.py reports --docs-page $(if $(MACHINE),--machine "$(MACHINE)") -o $(DOCS_PAGE)

docs-cases:
	$(PYTHON) cases_page.py -o $(CASES_PAGE)

check-docker: build-docker
	SHELLMATE_EVAL_ENGINE=docker REQUIRE_TOOLS=1 $(PYTHON) -I -m unittest discover -s .

check-podman: build-podman
	SHELLMATE_EVAL_ENGINE=podman REQUIRE_TOOLS=1 $(PYTHON) -I -m unittest discover -s .

test:
	$(PYTHON) -I -m unittest discover -s . -v

plan: check-config
	$(PYTHON) evals.py --config $(CONFIG) --plan

.PHONY: check-config
check-config:
	@test -f $(CONFIG) || { echo "$(CONFIG) not found: copy models.example.toml to $(CONFIG) and edit it" >&2; exit 1; }
