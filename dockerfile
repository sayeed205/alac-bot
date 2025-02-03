# FROM debian:latest AS deps-builder

# WORKDIR /build

# # Install essential packages
# RUN apt-get update && apt-get install -y \
#     curl \
#     wget \
#     git \
#     python3 \
#     unzip \
#     build-essential \
#     ca-certificates && rm -rf /var/lib/apt/lists/*

# # Install Bun
# RUN curl -fsSL https://bun.sh/install | bash
# ENV PATH="/root/.bun/bin:${PATH}"

# # Copy application files
# COPY package.json bun.lockb ./
# # COPY . .

# # Install Bun dependencies
# RUN bun i || true # Ignore errors
# RUN bun i

# # Stage 2: Go builder
# FROM golang:latest AS go-builder

# WORKDIR /build

# # Copy Go files
# COPY go.* .
# COPY wrapper.go .

# # Download dependencies
# RUN go mod tidy

# # Build the shared library
# RUN go build -o wrapper.so -buildmode=c-shared wrapper.go

# # Stage 3: Final runtime stage
# FROM oven/bun:latest

# WORKDIR /app

# # Install necessary packages for building glibc
# RUN apt-get update && apt-get install -y \
#     wget \
#     build-essential \
#     manpages-dev && rm -rf /var/lib/apt/lists/*

# # Build and install glibc 2.34
# RUN wget http://ftp.gnu.org/gnu/libc/glibc-2.34.tar.gz && tar -xvzf glibc-2.34.tar.gz && cd glibc-2.34 && mkdir build && cd build && ../configure --prefix=/opt/glibc-2.34 && make -j$(nproc) && make install && cd /app && rm -rf glibc-2.34*

# ENV LD_LIBRARY_PATH=/opt/glibc-2.34/lib:$LD_LIBRARY_PATH

# # Copy node_modules from deps builder
# COPY --from=deps-builder /build/node_modules ./node_modules

# # Copy the wrapper files from the Go build stage
# COPY --from=go-builder /build/wrapper.* ./

# # Copy application files
# COPY . .

# RUN ls -la

# CMD [ "bun", "dev" ]

# Build stage
FROM oven/bun:latest AS builder

# Install system dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    build-essential \
    ca-certificates \
    wget \
    curl && rm -rf /var/lib/apt/lists/*

# Install latest Go
RUN GO_VERSION=$(curl -s https://go.dev/VERSION?m=text | head -n1) && wget https://dl.google.com/go/${GO_VERSION}.linux-amd64.tar.gz -O go.tar.gz && tar -C /usr/local -xzf go.tar.gz && rm go.tar.gz
ENV PATH="/usr/local/go/bin:${PATH}"

WORKDIR /app

# Copy package files and Go source
COPY package.json bun.lock* ./
COPY go.* .
COPY wrapper.go .

# Install JS dependencies
RUN bun install || true
RUN bun install

# Build Go shared library
RUN go build -o wrapper.so -buildmode=c-shared wrapper.go

# Copy application files
COPY . .

# Remove Go source file from final build image
RUN rm wrapper.go

# Runtime stage
FROM oven/bun:latest

WORKDIR /app

RUN apt-get update && apt-get install -y \
    ca-certificates && rm -rf /var/lib/apt/lists/* && update-ca-certificates

# Copy built artifacts and dependencies
COPY --from=builder /app .

# Ensure wrapper files are present
COPY --from=builder /app/wrapper.* ./

# Run the application
CMD ["bun", "dev"]
