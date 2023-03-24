#!@shell@

set -e

docker="docker"
prog_docker_compose="docker-compose"
if which podman >/dev/null; then
  prog_docker="podman"
  prog_docker_compose="podman-compose"
fi
docker_compose=("$prog_docker_compose" "--file" "@composeFile@")

echo
echo "build and load docker image..."
echo

sudo "$docker" load <"@dockerImage@"
sudo "$docker" load <"@testScriptDockerImage@"

function cleanup {
  echo
  echo "docker compose down..."
  echo

  sudo "${docker_compose[@]}" down
}
trap "cleanup" SIGINT

echo
echo "docker compose up..."
echo

sudo "${docker_compose[@]}" up
