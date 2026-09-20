FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
       openssh-server sudo bash git ca-certificates curl wget tar gzip xz-utils unzip \
    && rm -rf /var/lib/apt/lists/* /etc/ssh/ssh_host_* \
    && useradd --uid 1000 --user-group --create-home --shell /bin/bash --password '*' developer \
    && install -d -o developer -g developer /home/developer/workspace \
    && install -d -m 0755 /etc/shroom \
    && passwd --lock root

COPY sshd_config /etc/ssh/sshd_config

WORKDIR /home/developer/workspace
# Core installs and launches shroom-ssh through SDK administration after boot.
CMD ["/bin/true"]
